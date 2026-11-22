import { createFileRoute } from "@tanstack/react-router";
import { InvitesPage } from "@/pages/admin/invites";

export const Route = createFileRoute("/_app/admin/invites")({
  component: InvitesPage,
});
