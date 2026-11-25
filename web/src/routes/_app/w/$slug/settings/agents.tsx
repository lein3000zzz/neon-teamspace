import { createFileRoute } from "@tanstack/react-router";
import { AgentsSettingsPage } from "@/pages/workspaces/settings/agents";

export const Route = createFileRoute("/_app/w/$slug/settings/agents")({
  component: AgentsSettingsPage,
});
