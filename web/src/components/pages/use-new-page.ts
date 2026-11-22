import { useNavigate } from "@tanstack/react-router";
import { useCreatePage, type PagePlace } from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { usePageNav } from "./page-nav-state";

/**
 * Makes an Untitled page in a place and opens it, its title ready to type,
 * with the page it is under opened in the tree so it shows there too.
 */
export function useNewPage(workspace: WorkspaceSummary, onNavigate?: () => void) {
  const nav = usePageNav();
  const navigate = useNavigate();
  const create = useCreatePage(workspace.id);
  const start = (place: PagePlace) =>
    create.mutate(place, {
      onSuccess: (page) => {
        nav.setNotice(null);
        if ("parent_id" in place) {
          nav.setOpen(place.parent_id, true);
        }
        onNavigate?.();
        void navigate({
          to: "/w/$slug/p/$pageId",
          params: { slug: workspace.slug, pageId: page.id },
        });
      },
      onError: (error) => nav.setNotice({ tone: "error", error }),
    });
  return { start, pending: create.isPending };
}
