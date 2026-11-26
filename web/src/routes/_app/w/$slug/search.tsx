import { createFileRoute } from "@tanstack/react-router";
import { readSearch, SearchPage, type SearchPageSearch } from "@/pages/workspaces/search";

/** The workspace's search, its query and filters in the address. */
export const Route = createFileRoute("/_app/w/$slug/search")({
  validateSearch: (search: Record<string, unknown>): SearchPageSearch => readSearch(search),
  component: SearchPage,
});
