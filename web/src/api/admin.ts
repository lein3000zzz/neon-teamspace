import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import type { Me, User } from "./account";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Invite = Schemas["Invite"];
export type CreatedInvite = Schemas["CreatedInvite"];
export type JoinRequest = Schemas["JoinRequest"];
export type Setting = Schemas["Setting"];

/** Accounts per page of the users list; the server takes up to 200. */
export const USERS_PAGE = 50;

/** Every account, a page at a time, in the order they were made. */
export const usersQuery = infiniteQueryOptions({
  queryKey: keys.users,
  queryFn: ({ pageParam, signal }) =>
    call(
      api.GET("/api/v1/admin/users", {
        params: {
          query: { limit: USERS_PAGE, ...(pageParam === undefined ? {} : { after: pageParam }) },
        },
        signal,
      }),
    ),
  initialPageParam: undefined as string | undefined,
  getNextPageParam: (page) => page.next ?? undefined,
});

/** Makes an account, whatever the registration mode. */
export function useCreateUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateUser"]) => call(api.POST("/api/v1/admin/users", { body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.users }),
  });
}

/**
 * Changes a role, disables or enables, or marks an address verified. When the
 * account is the admin's own, who they are is read again too.
 */
export function useUpdateUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: Schemas["UpdateUser"] & { id: string }) =>
      call(api.PATCH("/api/v1/admin/users/{id}", { params: { path: { id } }, body })),
    onSuccess: async (user: User) => {
      if (queryClient.getQueryData<Me | null>(keys.me)?.id === user.id) {
        await queryClient.invalidateQueries({ queryKey: keys.me });
      }
      await queryClient.invalidateQueries({ queryKey: keys.users });
    },
  });
}

/** Mails an unverified account a new verification link. */
export function useSendVerification() {
  return useMutation({
    mutationFn: (id: string) =>
      send(api.POST("/api/v1/admin/users/{id}/verification", { params: { path: { id } } })),
  });
}

export const invitesQuery = queryOptions({
  queryKey: keys.invites,
  queryFn: ({ signal }) => call(api.GET("/api/v1/admin/invites", { signal })),
});

/** Makes an invite, or mails one bound to an address. Its code is in this answer. */
export function useCreateInvite() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateInvite"]) =>
      call(api.POST("/api/v1/admin/invites", { body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.invites }),
  });
}

/** What keeps a typed code from making an invite, or null when it can. */
export function inviteCheckQuery(code: string) {
  return queryOptions({
    queryKey: keys.inviteCheck(code),
    queryFn: async ({ signal }) =>
      (await call(api.POST("/api/v1/admin/invites/check", { body: { code }, signal }))).problem ??
      null,
    enabled: code !== "",
    staleTime: 30 * 1000,
  });
}

/** An invite's code again, from its sealed copy. */
export function useInviteCode() {
  return useMutation({
    mutationFn: async (id: string) =>
      (await call(api.GET("/api/v1/admin/invites/{id}/code", { params: { path: { id } } }))).code,
  });
}

export function useRevokeInvite() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.POST("/api/v1/admin/invites/{id}/revoke", { params: { path: { id } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.invites }),
  });
}

export const joinRequestsQuery = queryOptions({
  queryKey: keys.joinRequests,
  queryFn: async ({ signal }) =>
    (await call(api.GET("/api/v1/admin/join-requests", { signal }))).requests,
});

/** Approves a request: an invite bound to the address, mailed there. */
export function useApproveJoin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      call(api.POST("/api/v1/admin/join-requests/{id}/approve", { params: { path: { id } } })),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: keys.joinRequests });
      await queryClient.invalidateQueries({ queryKey: keys.invites });
    },
  });
}

export function useDenyJoin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.POST("/api/v1/admin/join-requests/{id}/deny", { params: { path: { id } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.joinRequests }),
  });
}

export const settingsQuery = queryOptions({
  queryKey: keys.settings,
  queryFn: async ({ signal }) =>
    (await call(api.GET("/api/v1/admin/settings", { signal }))).settings,
});

/**
 * Saves setting changes by key, null going back to the default. All or
 * nothing: when one is invalid, none is saved and the error lists each. The
 * sign-in forms and the instance name read settings, so those are read again.
 */
export function useUpdateSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (changes: Record<string, unknown>) =>
      (
        await call(
          api.PATCH("/api/v1/admin/settings", {
            // The spec says only that it is an object, which the generated type reads as
            // one with no values; any JSON value per key is meant.
            body: { changes: changes as Schemas["UpdateSettings"]["changes"] },
          }),
        )
      ).settings,
    onSuccess: async (settings) => {
      queryClient.setQueryData(keys.settings, settings);
      await queryClient.invalidateQueries({ queryKey: keys.authConfig });
      await queryClient.invalidateQueries({ queryKey: keys.meta });
    },
  });
}
