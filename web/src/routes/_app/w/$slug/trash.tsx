import { createFileRoute } from "@tanstack/react-router";
import { TrashPage } from "@/pages/pages/trash";

/** The pages in the workspace's trash that the user could edit. */
export const Route = createFileRoute("/_app/w/$slug/trash")({
  component: TrashPage,
});
