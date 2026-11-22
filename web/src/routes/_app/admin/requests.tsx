import { createFileRoute } from "@tanstack/react-router";
import { RequestsPage } from "@/pages/admin/requests";

export const Route = createFileRoute("/_app/admin/requests")({
  component: RequestsPage,
});
