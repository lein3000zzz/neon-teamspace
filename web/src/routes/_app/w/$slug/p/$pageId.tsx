import { createFileRoute } from "@tanstack/react-router";
import { PageView, type PageSearch } from "@/pages/pages/page";

/**
 * A page by its id; the server answers not found for one the user may not see.
 * A database's page names the view it shows, and any page the row it shows
 * beside it, in the search.
 */
export const Route = createFileRoute("/_app/w/$slug/p/$pageId")({
  validateSearch: (search: Record<string, unknown>): PageSearch => ({
    ...(typeof search["view"] === "string" ? { view: search["view"] } : {}),
    ...(typeof search["peek"] === "string" ? { peek: search["peek"] } : {}),
  }),
  component: PageView,
});
