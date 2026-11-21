import { createFileRoute } from "@tanstack/react-router";
import { StyleguidePage } from "@/pages/styleguide";

export const Route = createFileRoute("/styleguide")({
  component: StyleguidePage,
});
