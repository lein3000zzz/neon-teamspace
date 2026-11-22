import { createFileRoute } from "@tanstack/react-router";
import { InvitationsPage } from "@/pages/account/invitations";

export const Route = createFileRoute("/_app/account/invitations")({
  component: InvitationsPage,
});
