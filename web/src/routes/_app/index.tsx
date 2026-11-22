import { createFileRoute } from "@tanstack/react-router";
import { StartPage } from "@/pages/workspaces/start";

/** Opens the workspace used last, or the way into one. */
export const Route = createFileRoute("/_app/")({
  component: StartPage,
});
