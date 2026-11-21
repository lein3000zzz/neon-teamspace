import { createFileRoute } from "@tanstack/react-router";
import { AppearancePage } from "@/pages/appearance";

export const Route = createFileRoute("/settings/appearance")({
  component: AppearancePage,
});
