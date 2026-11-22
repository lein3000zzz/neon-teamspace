import { useQuery } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import { createContext, use } from "react";
import { workspacesQuery, type WorkspaceSummary } from "@/api/workspaces";
import { useMe } from "@/components/shell/signed-in";
import { lastWorkspaceId, pickWorkspace } from "@/lib/last-workspace";

/** The workspace the page at /w/{slug} is in, as the user's list of workspaces has it. */
export const WorkspaceContext = createContext<WorkspaceSummary | null>(null);

/** The workspace of the page, inside the pages under /w/{slug}. */
export function useWorkspace(): WorkspaceSummary {
  const workspace = use(WorkspaceContext);
  if (workspace === null) {
    throw new Error("useWorkspace must be used inside a workspace's pages");
  }
  return workspace;
}

/**
 * The workspace the sidebar shows: the one in the address, under /w/{slug};
 * elsewhere, such as on the account pages, the one used last, or the first.
 * With the list of workspaces, for the switcher.
 */
export function useCurrentWorkspace() {
  const me = useMe();
  const list = useQuery(workspacesQuery);
  const slug = useParams({ strict: false, select: (params) => params.slug });
  const workspaces = list.data?.workspaces ?? [];
  const current =
    slug === undefined
      ? pickWorkspace(workspaces, lastWorkspaceId(me.id))
      : workspaces.find((workspace) => workspace.slug === slug);
  return { list, current };
}
