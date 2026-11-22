import { createFileRoute } from "@tanstack/react-router";
import { SessionsPage } from "@/pages/account/sessions";

export const Route = createFileRoute("/_app/account/sessions")({
  component: SessionsPage,
});
