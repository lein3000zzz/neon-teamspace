import { createFileRoute } from "@tanstack/react-router";
import { GeneralSettingsPage } from "@/pages/workspaces/settings/general";

export const Route = createFileRoute("/_app/w/$slug/settings/")({
  component: GeneralSettingsPage,
});
