import { createFileRoute } from "@tanstack/react-router";
import { ConnectedAppsPage } from "@/pages/account/apps";

export const Route = createFileRoute("/_app/account/apps")({
  component: ConnectedAppsPage,
});
