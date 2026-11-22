import { createFileRoute } from "@tanstack/react-router";
import { MembersSettingsPage } from "@/pages/workspaces/settings/members";

export const Route = createFileRoute("/_app/w/$slug/settings/members")({
  component: MembersSettingsPage,
});
