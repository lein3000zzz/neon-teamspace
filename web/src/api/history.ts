import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { call } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Version = Schemas["Version"];
export type VersionList = Schemas["VersionList"];
export type PageVersion = Schemas["PageVersion"];
export type Comparison = Schemas["Comparison"];
export type BlockChange = Schemas["BlockChange"];
export type BlockChangeKind = Schemas["BlockChangeKind"];
export type RestoredPage = Schemas["RestoredPage"];

/** How many versions a part of a page's history holds. */
export const VERSIONS_PART = 50;

const versionPath = (pageId: string, versionId: string) => ({
  path: { id: pageId, version_id: versionId },
});

/** A page's versions, the latest first, a part at a time. */
export function versionsQuery(workspaceId: string, pageId: string) {
  return infiniteQueryOptions({
    queryKey: keys.versions(workspaceId, pageId),
    queryFn: ({ pageParam, signal }) =>
      call(
        api.GET("/api/v1/pages/{id}/versions", {
          params: {
            path: { id: pageId },
            query: {
              limit: VERSIONS_PART,
              ...(pageParam === undefined ? {} : { after: pageParam }),
            },
          },
          signal,
        }),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next ?? undefined,
  });
}

/** A version with its blocks, which never changes. */
export function versionQuery(workspaceId: string, pageId: string, versionId: string) {
  return queryOptions({
    queryKey: keys.version(workspaceId, pageId, versionId),
    queryFn: ({ signal }) =>
      call(
        api.GET("/api/v1/pages/{id}/versions/{version_id}", {
          params: versionPath(pageId, versionId),
          signal,
        }),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * A version compared with the page as it is now, with the revision a restore
 * names. Read afresh each time it is shown, as the page changes under it.
 */
export function comparisonQuery(workspaceId: string, pageId: string, versionId: string) {
  return queryOptions({
    queryKey: keys.comparison(workspaceId, pageId, versionId),
    queryFn: ({ signal }) =>
      call(
        api.GET("/api/v1/pages/{id}/versions/{version_id}/compare", {
          params: versionPath(pageId, versionId),
          signal,
        }),
      ),
    staleTime: 0,
  });
}

/**
 * Restores a version, on the revision of the comparison the user saw: the
 * server refuses it with edit_conflict once the page changed since. The
 * editor shows the result through the live document; the history and the
 * comparisons are read again.
 */
export function useRestoreVersion(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ versionId, base }: { versionId: string; base: string }) =>
      call(
        api.POST("/api/v1/pages/{id}/versions/{version_id}/restore", {
          params: versionPath(pageId, versionId),
          body: { base },
        }),
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: keys.versions(workspaceId, pageId) }),
  });
}
