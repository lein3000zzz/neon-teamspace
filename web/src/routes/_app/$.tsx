import { createFileRoute } from "@tanstack/react-router";
import { NotFoundPage } from "@/pages/not-found";

/** Any address no route claims, answered inside the shell so the navigation stays at hand. */
export const Route = createFileRoute("/_app/$")({
  component: NotFoundPage,
});
