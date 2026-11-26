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

export type Notification = Schemas["Notification"];
export type NotificationKind = Schemas["NotificationKind"];
export type NotificationSettings = Schemas["NotificationSettings"];
export type MailPreference = Schemas["MailPreference"];

/** Which notifications a part of the inbox lists. */
export type InboxFilter = "unread" | "all" | "archived";

/** Notifications per part of the inbox's list. */
export const INBOX_PART = 50;

/** The user's notifications, from every workspace, newest first, a part at a time. */
export function inboxQuery(filter: InboxFilter) {
  return infiniteQueryOptions({
    queryKey: keys.inboxList(filter),
    queryFn: ({ pageParam, signal }) =>
      call(
        api.GET("/api/v1/inbox", {
          params: {
            query: {
              filter,
              limit: INBOX_PART,
              ...(pageParam === undefined ? {} : { before: pageParam }),
            },
          },
          signal,
        }),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (part) => part.next ?? undefined,
  });
}

/** How many notifications wait unread, the sidebar's count. */
export const inboxCountQuery = queryOptions({
  queryKey: keys.inboxCount,
  queryFn: async ({ signal }) => (await call(api.GET("/api/v1/inbox/count", { signal }))).unread,
});

/** Reads the inbox again, its parts and its count, after a change made here. */
function refreshNotifications(queryClient: QueryClient) {
  void queryClient.invalidateQueries({ queryKey: keys.inbox });
}

/**
 * Reads the inbox again, its parts, its count and the invitations it shows,
 * as its live feed tells of a change made elsewhere.
 */
export function refreshInbox(queryClient: QueryClient) {
  refreshNotifications(queryClient);
  void queryClient.invalidateQueries({ queryKey: keys.myInvitations });
}

/** Marks one notification read or unread, archived or not. */
export function useMarkNotification() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string; read?: boolean; archived?: boolean }) =>
      send(api.PATCH("/api/v1/notifications/{id}", { params: { path: { id } }, body })),
    onSuccess: () => refreshNotifications(queryClient),
  });
}

/** Marks every notification read. */
export function useMarkAllRead() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => call(api.POST("/api/v1/inbox/read", { body: {} })),
    onSuccess: () => refreshNotifications(queryClient),
  });
}

/** How the user wants mail of their inbox, and whether the instance sends it. */
export const notificationSettingsQuery = queryOptions({
  queryKey: keys.notificationSettings,
  queryFn: ({ signal }) => call(api.GET("/api/v1/me/notifications", { signal })),
});

export function useSetMail() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (mail: MailPreference) =>
      call(api.PUT("/api/v1/me/notifications", { body: { mail } })),
    onSuccess: (settings) => queryClient.setQueryData(keys.notificationSettings, settings),
  });
}

/** Stops notification mail with the token of a mail's link, without signing in. */
export function useUnsubscribe() {
  return useMutation({
    mutationFn: (token: string) =>
      send(api.POST("/api/v1/unsubscribe", { params: { query: { token } } })),
  });
}

/**
 * Where a notification opens: its page, at the thread or block it is about.
 * `undefined` for one with no page, an invitation.
 */
export function notificationTarget(
  notification: Notification,
): { slug: string; pageId: string; search: { comment?: string; block?: string } } | undefined {
  const page = notification.page ?? undefined;
  if (page === undefined) {
    return undefined;
  }
  const thread = notification.thread_id ?? undefined;
  const block = notification.block_id ?? undefined;
  return {
    slug: notification.workspace.slug,
    pageId: page.id,
    search: thread !== undefined ? { comment: thread } : block !== undefined ? { block } : {},
  };
}
