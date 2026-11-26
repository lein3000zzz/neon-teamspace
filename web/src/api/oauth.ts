import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type AuthorizationParams = Schemas["AuthorizationParams"];
export type ConsentInfo = Schemas["ConsentInfo"];
export type ConsentOption = Schemas["ConsentOption"];
export type ConnectedApp = Schemas["ConnectedAppInfo"];

/** The parameters of an authorization request the consent page reads from its address. */
export const AUTHORIZATION_PARAMS = [
  "response_type",
  "client_id",
  "redirect_uri",
  "code_challenge",
  "code_challenge_method",
  "scope",
  "state",
  "resource",
] as const satisfies readonly (keyof AuthorizationParams)[];

/**
 * What a client asks to be allowed, checked again by the server, and what the
 * page may offer. Read once: the request does not change while the page is
 * open, and a problem with it is the answer, not a reason to ask again.
 */
export function consentQuery(request: AuthorizationParams) {
  return queryOptions({
    queryKey: keys.consent(request),
    queryFn: () => call(api.POST("/api/v1/oauth/consent", { body: request })),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

/** Allows the client what the person chose; the answer says where the browser goes. */
export function useApprove() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["ApproveConsent"]) =>
      call(api.POST("/api/v1/oauth/consent/approve", { body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.connectedApps }),
  });
}

/** Tells the client the person said no; the answer says where the browser goes. */
export function useDeny() {
  return useMutation({
    mutationFn: (request: AuthorizationParams) =>
      call(api.POST("/api/v1/oauth/consent/deny", { body: { request } })),
  });
}

/** The apps the person connected, as themselves or as their agents. */
export const connectedAppsQuery = queryOptions({
  queryKey: keys.connectedApps,
  queryFn: async ({ signal }) =>
    (await call(api.GET("/api/v1/account/connected-apps", { signal }))).apps,
});

export function useDisconnectApp() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.DELETE("/api/v1/account/connected-apps/{id}", { params: { path: { id } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.connectedApps }),
  });
}

/** The apps acting as the workspace's agents that the person may see and revoke. */
export function workspaceAppsQuery(workspaceId: string) {
  return queryOptions({
    queryKey: keys.workspaceApps(workspaceId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/connected-apps", {
            params: { path: { id: workspaceId } },
            signal,
          }),
        )
      ).apps,
  });
}

export function useDisconnectWorkspaceApp(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (appId: string) =>
      send(
        api.DELETE("/api/v1/workspaces/{id}/connected-apps/{app_id}", {
          params: { path: { id: workspaceId, app_id: appId } },
        }),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.workspaceApps(workspaceId) });
      void queryClient.invalidateQueries({ queryKey: keys.connectedApps });
    },
  });
}
