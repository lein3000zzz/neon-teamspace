import { queryOptions, useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import type { ThemePreferences } from "@/theme/preferences";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { withDeadline } from "./deadline";
import { ApiError } from "./errors";
import { keys, PRIVATE_ROOTS } from "./keys";
import { META_TIMEOUT_MS } from "./meta";

export type Me = Schemas["Me"];
export type User = Schemas["User"];
export type SessionInfo = Schemas["SessionInfo"];
export type ApiToken = Schemas["ApiToken"];
export type Scope = Schemas["Scope"];

/**
 * Who is signed in, or null for nobody. A 401 is the answer for a visitor
 * without a session, so it is data here, not an error. Every signed-in page
 * waits for it, so it gets the same deadline as the instance's name: a server
 * that never answers shows as unreachable, with a retry, not as a blank page.
 */
export const meQuery = queryOptions({
  queryKey: keys.me,
  queryFn: ({ signal }): Promise<Me | null> =>
    withDeadline(signal, META_TIMEOUT_MS, async (deadline) => {
      const { data, error, response } = await api.GET("/api/v1/account/me", { signal: deadline });
      if (response.status === 401) {
        return null;
      }
      if (data === undefined) {
        throw ApiError.from(response, error);
      }
      return data;
    }),
  staleTime: 5 * 60 * 1000,
});

/**
 * Drops everything that belonged to whoever was signed in, but who that is:
 * the signed-in layout watches that query, and removing it would make the
 * mounted pages fetch it again at once. Every mutation goes too, with the
 * passwords, token secrets and invite codes their variables and answers hold;
 * a component still showing one keeps its own copy until it unmounts.
 */
export function forgetPrivateData(queryClient: QueryClient) {
  queryClient.removeQueries({
    predicate: (query) =>
      PRIVATE_ROOTS.some((root) => query.queryKey[0] === root) &&
      !(query.queryKey[0] === keys.me[0] && query.queryKey[1] === keys.me[1]),
  });
  queryClient.getMutationCache().clear();
}

/**
 * After signing in: forgets what belonged to the previous user, then asks who
 * is signed in now, so nothing of one account shows to the next.
 */
export async function refreshSession(queryClient: QueryClient) {
  forgetPrivateData(queryClient);
  await queryClient.fetchQuery({ ...meQuery, staleTime: 0 });
}

export function useUpdateProfile() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["UpdateMe"]) => call(api.PATCH("/api/v1/account/me", { body })),
    onSuccess: (user) =>
      queryClient.setQueryData<Me | null>(keys.me, (me) => (me ? { ...me, ...user } : me)),
  });
}

/** Changes the password; the server signs out every other session. */
export function useChangePassword() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["ChangePassword"]) =>
      send(api.POST("/api/v1/account/password", { body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.sessions }),
  });
}

export const sessionsQuery = queryOptions({
  queryKey: keys.sessions,
  queryFn: async ({ signal }) =>
    (await call(api.GET("/api/v1/account/sessions", { signal }))).sessions,
});

export function useRevokeSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.DELETE("/api/v1/account/sessions/{id}", { params: { path: { id } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.sessions }),
  });
}

/** Ends every session but this one; answers how many ended. */
export function useRevokeOtherSessions() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => (await call(api.DELETE("/api/v1/account/sessions"))).revoked,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.sessions }),
  });
}

export const tokensQuery = queryOptions({
  queryKey: keys.tokens,
  queryFn: async ({ signal }) => (await call(api.GET("/api/v1/account/tokens", { signal }))).tokens,
});

/** Makes a token. Its secret is in this answer only, and never again. */
export function useCreateToken() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateApiToken"]) =>
      call(api.POST("/api/v1/account/tokens", { body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.tokens }),
  });
}

export function useRevokeToken() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.DELETE("/api/v1/account/tokens/{id}", { params: { path: { id } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.tokens }),
  });
}

/**
 * The look of the app kept with the account. Read once per signed-in user and
 * not refetched on focus: the theme store is the live copy, and a refetch
 * landing while someone drags the accent slider would pull it back.
 */
export function preferencesQuery(userId: string) {
  return queryOptions({
    queryKey: keys.preferences(userId),
    queryFn: async ({ signal }) =>
      fromApiPreferences(await call(api.GET("/api/v1/account/preferences", { signal }))),
    staleTime: Number.POSITIVE_INFINITY,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}

/** Saves the look to the account. The cache is left alone, for the reason above. */
export function useSavePreferences() {
  return useMutation({
    mutationFn: (preferences: ThemePreferences) =>
      call(api.PUT("/api/v1/account/preferences", { body: toApiPreferences(preferences) })).then(
        fromApiPreferences,
      ),
  });
}

export function fromApiPreferences(preferences: Schemas["Preferences"]): ThemePreferences {
  return {
    palette: preferences.palette,
    mode: preferences.mode,
    accentHue: preferences.accent_hue,
    uiFont: preferences.ui_font,
  };
}

export function toApiPreferences(preferences: ThemePreferences): Schemas["Preferences"] {
  return {
    palette: preferences.palette,
    mode: preferences.mode,
    accent_hue: preferences.accentHue,
    ui_font: preferences.uiFont,
  };
}
