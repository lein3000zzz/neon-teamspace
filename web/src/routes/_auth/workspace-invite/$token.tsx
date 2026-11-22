import { createFileRoute } from "@tanstack/react-router";
import { WorkspaceInviteLinkPage } from "@/pages/auth/workspace-invite";

/** A workspace invite link; the page takes the token out of the address. */
export const Route = createFileRoute("/_auth/workspace-invite/$token")({
  component: WorkspaceInviteLinkPage,
});
