import { createFileRoute } from "@tanstack/react-router";
import { NotificationsPage } from "@/pages/account/notifications";

export const Route = createFileRoute("/_app/account/notifications")({
  component: NotificationsPage,
});
