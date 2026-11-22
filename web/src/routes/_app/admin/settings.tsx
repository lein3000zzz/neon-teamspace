import { createFileRoute } from "@tanstack/react-router";
import { SettingsPage } from "@/pages/admin/settings";

export const Route = createFileRoute("/_app/admin/settings")({
  component: SettingsPage,
});
