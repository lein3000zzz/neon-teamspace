import { createFileRoute } from "@tanstack/react-router";
import { AppearancePage } from "@/pages/account/appearance";

export const Route = createFileRoute("/_app/account/appearance")({
  component: AppearancePage,
});
