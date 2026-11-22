import { createFileRoute } from "@tanstack/react-router";
import { JoinPage } from "@/pages/auth/join";

export const Route = createFileRoute("/_auth/join/")({
  component: JoinPage,
});
