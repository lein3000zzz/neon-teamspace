import { infiniteQueryOptions, keepPreviousData, queryOptions } from "@tanstack/react-query";
import { call } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";
import type { PageKind } from "./pages";

export type SearchResults = Schemas["SearchResults"];
export type SearchResult = Schemas["SearchResult"];
export type TextRun = Schemas["TextRun"];

/** What a search asks besides its query: where, what, by whom and how recently. */
export type SearchFilters = {
  teamspace_id?: string;
  kind?: PageKind;
  created_by?: string;
  edited_by?: string;
  /** A duration as the server reads them, such as 7d. */
  edited_within?: string;
};

/** How many results the quick search shows, and the search page reads at a time. */
export const QUICK_RESULTS = 8;
export const SEARCH_PART = 20;

/** How many of the pages changed last the quick search offers. */
export const RECENT_PAGES = 8;

/** Searches and the pages changed last are read again after this long. */
const SEARCH_STALE_MS = 10 * 1000;

const searchPath = (workspaceId: string) => ({ path: { id: workspaceId } });

/**
 * The best few results for what is typed in the quick search. The last
 * results stay shown while the next are read, so the list does not flicker as
 * one types.
 */
export function quickSearchQuery(workspaceId: string, query: string) {
  return queryOptions({
    queryKey: keys.search(workspaceId, { query, limit: QUICK_RESULTS }),
    queryFn: ({ signal }) =>
      call(
        api.GET("/api/v1/workspaces/{id}/search", {
          params: { ...searchPath(workspaceId), query: { query, limit: QUICK_RESULTS } },
          signal,
        }),
      ),
    placeholderData: keepPreviousData,
    staleTime: SEARCH_STALE_MS,
  });
}

/** A search's results a part at a time, the best first, for the search page. */
export function searchQuery(workspaceId: string, query: string, filters: SearchFilters) {
  const asked = { query, ...filters };
  return infiniteQueryOptions({
    queryKey: keys.search(workspaceId, asked),
    queryFn: ({ pageParam, signal }) =>
      call(
        api.GET("/api/v1/workspaces/{id}/search", {
          params: {
            ...searchPath(workspaceId),
            query: {
              ...asked,
              limit: SEARCH_PART,
              ...(pageParam === undefined ? {} : { after: pageParam }),
            },
          },
          signal,
        }),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next ?? undefined,
    staleTime: SEARCH_STALE_MS,
  });
}

/** The pages and databases the user sees that changed last, rows left out. */
export function recentPagesQuery(workspaceId: string) {
  return queryOptions({
    queryKey: keys.recentPages(workspaceId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/recent-pages", {
            params: { ...searchPath(workspaceId), query: { limit: RECENT_PAGES } },
            signal,
          }),
        )
      ).pages,
    staleTime: SEARCH_STALE_MS,
  });
}
