import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Page = Schemas["Page"];
export type TreePage = Schemas["TreePage"];
export type PageTree = Schemas["PageTree"];
export type PageAccess = Schemas["PageAccess"];
export type PageStyle = Schemas["PageStyle"];
export type StyleChange = Schemas["StyleChange"];
export type Font = Schemas["Font"];
export type Cover = Schemas["Cover"];
export type Placement = Schemas["Placement"];
export type Crumb = Schemas["Crumb"];
export type Share = Schemas["Share"];
export type ShareAccess = Schemas["ShareAccess"];
export type CreatePage = Schemas["CreatePage"];
export type UpdatePage = Schemas["UpdatePage"];
export type MovePage = Schemas["MovePage"];

/** Where a page goes: under a page, at the top of a teamspace, or among the caller's private pages. */
export type PagePlace = { parent_id: string } | { teamspace_id: string } | { private: true };

/** Pages in the trash per part of its list. */
export const TRASH_PART = 50;

const pagePath = (id: string) => ({ params: { path: { id } } });

/**
 * Reads every part of a tree: one part holds up to 1000 pages, so most trees
 * come in one answer, and a larger one is read part after part.
 */
async function wholeTree(
  part: (after: string | undefined) => Promise<PageTree>,
  after?: string,
): Promise<TreePage[]> {
  const answer = await part(after);
  const next = answer.next ?? undefined;
  return next === undefined ? answer.pages : [...answer.pages, ...(await wholeTree(part, next))];
}

const afterQuery = (after: string | undefined) => (after === undefined ? {} : { after });

/** Trees change with every page made, moved or renamed; they are read again on those. */
const TREE_STALE_MS = 30 * 1000;

/** A teamspace's pages, each before the pages under it, siblings in order. */
export function teamspacePagesQuery(workspaceId: string, teamspaceId: string) {
  return queryOptions({
    queryKey: keys.teamspacePages(workspaceId, teamspaceId),
    queryFn: ({ signal }) =>
      wholeTree((after) =>
        call(
          api.GET("/api/v1/teamspaces/{id}/pages", {
            params: { path: { id: teamspaceId }, query: afterQuery(after) },
            signal,
          }),
        ),
      ),
    staleTime: TREE_STALE_MS,
  });
}

/** The signed-in member's private pages in a workspace. Not for guests, who have none. */
export function privatePagesQuery(workspaceId: string) {
  return queryOptions({
    queryKey: keys.privatePages(workspaceId),
    queryFn: ({ signal }) =>
      wholeTree((after) =>
        call(
          api.GET("/api/v1/workspaces/{id}/private-pages", {
            params: { path: { id: workspaceId }, query: afterQuery(after) },
            signal,
          }),
        ),
      ),
    staleTime: TREE_STALE_MS,
  });
}

/**
 * The pages shared with the signed-in user, with the pages under them. The
 * topmost come without a parent, as the page above them stays hidden.
 */
export function sharedPagesQuery(workspaceId: string) {
  return queryOptions({
    queryKey: keys.sharedPages(workspaceId),
    queryFn: ({ signal }) =>
      wholeTree((after) =>
        call(
          api.GET("/api/v1/workspaces/{id}/shared-pages", {
            params: { path: { id: workspaceId }, query: afterQuery(after) },
            signal,
          }),
        ),
      ),
    staleTime: TREE_STALE_MS,
  });
}

/** One page, without its content. A page in the trash shows only to those who edit it. */
export function pageQuery(workspaceId: string, pageId: string) {
  return queryOptions({
    queryKey: keys.page(workspaceId, pageId),
    queryFn: ({ signal }) => call(api.GET("/api/v1/pages/{id}", { ...pagePath(pageId), signal })),
  });
}

/** The way down to a page: the highest page above it the user sees first, the page last. */
export function breadcrumbsQuery(workspaceId: string, pageId: string) {
  return queryOptions({
    queryKey: keys.breadcrumbs(workspaceId, pageId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/pages/{id}/breadcrumbs", {
            ...pagePath(pageId),
            signal,
          }),
        )
      ).pages,
  });
}

/** The page's own shares first, then those set on each page above it. Not for guests. */
export function sharesQuery(workspaceId: string, pageId: string) {
  return queryOptions({
    queryKey: keys.shares(workspaceId, pageId),
    queryFn: async ({ signal }) =>
      (await call(api.GET("/api/v1/pages/{id}/shares", { ...pagePath(pageId), signal }))).shares,
  });
}

/** The pages in the trash the user could edit, latest first, a part at a time. */
export function trashQuery(workspaceId: string) {
  return infiniteQueryOptions({
    queryKey: keys.trash(workspaceId),
    queryFn: ({ pageParam, signal }) =>
      call(
        api.GET("/api/v1/workspaces/{id}/trash", {
          params: {
            path: { id: workspaceId },
            query: { limit: TRASH_PART, ...afterQuery(pageParam) },
          },
          signal,
        }),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (part) => part.next ?? undefined,
  });
}

/** The user's favorites in a workspace, in their order. */
export function favoritesQuery(workspaceId: string) {
  return queryOptions({
    queryKey: keys.favorites(workspaceId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/account/favorites", {
            params: { query: { workspace_id: workspaceId } },
            signal,
          }),
        )
      ).pages,
    staleTime: TREE_STALE_MS,
  });
}

/** What the tree rows and the way down to a page show of it. */
type Shown = { title?: string; icon?: string | null };

/**
 * Writes a page's new title or icon into every list that shows it: the trees,
 * the favorites, the ways down to pages and the page itself. The title shows
 * there as it is typed, before it is saved.
 */
export function showPage(queryClient: QueryClient, workspaceId: string, id: string, shown: Shown) {
  const apply = <T extends { id: string; title: string; icon?: string | null }>(entry: T): T =>
    entry.id === id ? { ...entry, ...shown } : entry;
  queryClient.setQueriesData<TreePage[]>({ queryKey: keys.pageTrees(workspaceId) }, (pages) =>
    pages?.map(apply),
  );
  queryClient.setQueryData<Page[]>(keys.favorites(workspaceId), (pages) => pages?.map(apply));
  queryClient.setQueriesData<Crumb[]>(
    {
      queryKey: keys.pages(workspaceId),
      predicate: (query) => query.queryKey[4] === "breadcrumbs",
    },
    (crumbs) => crumbs?.map(apply),
  );
  queryClient.setQueryData<Page>(keys.page(workspaceId, id), (page) => page && apply(page));
}

/**
 * Reads again everything a page's place shows in: the trees, the favorites,
 * the trash, and each page read, as a move or a trash takes pages under it
 * along. Not awaited, for the same reason as the workspace's refresh.
 */
function refreshPlaces(queryClient: QueryClient, workspaceId: string) {
  void queryClient.invalidateQueries({ queryKey: keys.pageTrees(workspaceId) });
  void queryClient.invalidateQueries({ queryKey: keys.favorites(workspaceId) });
  void queryClient.invalidateQueries({ queryKey: keys.trash(workspaceId) });
  void queryClient.invalidateQueries({ queryKey: keys.pages(workspaceId) });
}

/** Makes a page; Untitled unless a title is given, last among its siblings unless placed. */
export function useCreatePage(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: CreatePage) =>
      call(api.POST("/api/v1/workspaces/{id}/pages", { ...pagePath(workspaceId), body })),
    onSuccess: (page) => {
      queryClient.setQueryData(keys.page(workspaceId, page.id), page);
      void queryClient.invalidateQueries({ queryKey: keys.pageTrees(workspaceId) });
    },
  });
}

/**
 * Changes a page's title, icon, cover or style. What it shows in lists is
 * written there as soon as the server answers, without reading them again. A
 * title typed further while the save was on its way stays: the next save
 * sends it.
 */
export function useUpdatePage(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: UpdatePage) =>
      call(api.PATCH("/api/v1/pages/{id}", { ...pagePath(pageId), body })),
    onSuccess: (page, body) => {
      const shown = queryClient.getQueryData<Page>(keys.page(workspaceId, page.id));
      const typedSince =
        typeof body.title === "string" && shown !== undefined && shown.title !== body.title;
      const next = typedSince ? { ...page, title: shown.title } : page;
      queryClient.setQueryData(keys.page(workspaceId, page.id), next);
      showPage(queryClient, workspaceId, page.id, { title: next.title, icon: next.icon ?? null });
    },
  });
}

/** Moves a page, with the pages under it, to another place or among its siblings. */
export function useMovePage(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: MovePage & { id: string }) =>
      call(api.POST("/api/v1/pages/{id}/move", { ...pagePath(id), body })),
    onSuccess: () => refreshPlaces(queryClient, workspaceId),
  });
}

/** Moves a page to the trash, with the pages under it. */
export function useTrashPage(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => call(api.POST("/api/v1/pages/{id}/trash", pagePath(id))),
    onSuccess: () => refreshPlaces(queryClient, workspaceId),
  });
}

/** Brings a page back from the trash, with the pages that went there with it. */
export function useRestorePage(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => call(api.POST("/api/v1/pages/{id}/restore", pagePath(id))),
    onSuccess: () => refreshPlaces(queryClient, workspaceId),
  });
}

/**
 * Deletes a page in the trash for good, with everything under it. `onDeleted`
 * runs before the page leaves the cache, so a view of it moves on first, as
 * for a workspace left.
 */
export function useDeletePage(workspaceId: string, onDeleted?: (id: string) => void) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => send(api.DELETE("/api/v1/pages/{id}", pagePath(id))),
    onSuccess: (_, id) => {
      onDeleted?.(id);
      queryClient.removeQueries({ queryKey: keys.page(workspaceId, id) });
      void queryClient.invalidateQueries({ queryKey: keys.trash(workspaceId) });
      void queryClient.invalidateQueries({ queryKey: keys.favorites(workspaceId) });
    },
  });
}

/**
 * Shares on a page and the pages under it change together: the list of each
 * page read below it holds the share as inherited.
 */
function refreshShares(queryClient: QueryClient, workspaceId: string) {
  void queryClient.invalidateQueries({
    queryKey: keys.pages(workspaceId),
    predicate: (query) => query.queryKey[4] === "shares",
  });
}

export function useAddShare(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateShare"]) =>
      call(api.POST("/api/v1/pages/{id}/shares", { ...pagePath(pageId), body })),
    onSuccess: () => refreshShares(queryClient, workspaceId),
  });
}

export function useChangeShare(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ userId, access }: { userId: string; access: ShareAccess }) =>
      call(
        api.PATCH("/api/v1/pages/{id}/shares/{user_id}", {
          params: { path: { id: pageId, user_id: userId } },
          body: { access },
        }),
      ),
    onSuccess: () => refreshShares(queryClient, workspaceId),
  });
}

export function useRemoveShare(workspaceId: string, pageId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) =>
      send(
        api.DELETE("/api/v1/pages/{id}/shares/{user_id}", {
          params: { path: { id: pageId, user_id: userId } },
        }),
      ),
    onSuccess: () => refreshShares(queryClient, workspaceId),
  });
}

export function useAddFavorite(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (pageId: string) =>
      call(api.POST("/api/v1/account/favorites", { body: { page_id: pageId } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.favorites(workspaceId) }),
  });
}

/** Moves a favorite among the others, the placement naming them. */
export function useMoveFavorite(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ pageId, placement }: { pageId: string; placement: Placement }) =>
      send(
        api.POST("/api/v1/account/favorites/{page_id}/move", {
          params: { path: { page_id: pageId } },
          body: { placement },
        }),
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.favorites(workspaceId) }),
  });
}

export function useRemoveFavorite(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (pageId: string) =>
      send(
        api.DELETE("/api/v1/account/favorites/{page_id}", {
          params: { path: { page_id: pageId } },
        }),
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.favorites(workspaceId) }),
  });
}
