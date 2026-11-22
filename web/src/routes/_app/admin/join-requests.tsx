import { createFileRoute } from "@tanstack/react-router";
import { RequestsPage } from "@/pages/admin/requests";

/** The requests to join; the mail telling admins of a new one links here. */
export const Route = createFileRoute("/_app/admin/join-requests")({
  component: RequestsPage,
});
