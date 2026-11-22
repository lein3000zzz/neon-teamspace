import { createFileRoute } from "@tanstack/react-router";
import { ForgotPage } from "@/pages/auth/forgot";

export const Route = createFileRoute("/_auth/forgot")({
  component: ForgotPage,
});
