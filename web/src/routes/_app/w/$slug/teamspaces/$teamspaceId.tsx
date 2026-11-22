import { createFileRoute } from "@tanstack/react-router";
import { TeamspacePage } from "@/pages/workspaces/teamspace";

/** A teamspace by its id; the server answers not found for one the user may not see. */
export const Route = createFileRoute("/_app/w/$slug/teamspaces/$teamspaceId")({
  component: TeamspacePage,
});
