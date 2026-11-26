import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { WebhooksSettingsPage } from "@/pages/workspaces/settings/webhooks";

export const Route = createFileRoute("/_app/w/$slug/settings/webhooks")({
  validateSearch: (search: Record<string, unknown>): { for?: string } => {
    const identity = searchText(search["for"]);
    return identity === undefined ? {} : { for: identity };
  },
  component: WebhooksSettingsPage,
});
