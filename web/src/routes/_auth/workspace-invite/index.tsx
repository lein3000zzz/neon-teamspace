import { createFileRoute } from "@tanstack/react-router";
import { WorkspaceInvitePage } from "@/pages/auth/workspace-invite";

/** Where an invite link's page carries on once its token is out of the address. */
export const Route = createFileRoute("/_auth/workspace-invite/")({
  component: WorkspaceInvitePage,
});
