import { useNavigate } from "@tanstack/react-router";
import { useCreateDatabase } from "@/api/databases";
import type { PagePlace } from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { usePageNav } from "./page-nav-state";

/**
 * Makes an Untitled database in a place, a page of its own there with a
 * table and a board, and opens it, with the page it is under opened in the
 * tree so it shows there too.
 */
export function useNewDatabase(workspace: WorkspaceSummary, onNavigate?: () => void) {
  const nav = usePageNav();
  const navigate = useNavigate();
  const create = useCreateDatabase(workspace.id);
  const start = (place: PagePlace) =>
    create.mutate(place, {
      onSuccess: (database) => {
        nav.setNotice(null);
        if ("parent_id" in place) {
          nav.setOpen(place.parent_id, true);
        }
        onNavigate?.();
        void navigate({
          to: "/w/$slug/p/$pageId",
          params: { slug: workspace.slug, pageId: database.page.id },
          search: {},
        });
      },
      onError: (error) => nav.setNotice({ tone: "error", error }),
    });
  return { start, pending: create.isPending };
}
