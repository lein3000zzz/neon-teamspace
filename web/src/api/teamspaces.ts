import { queryOptions, useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Teamspace = Schemas["Teamspace"];
export type TeamspaceList = Schemas["TeamspaceList"];
export type TeamspaceName = Schemas["TeamspaceName"];
export type TeamspaceAccess = Schemas["TeamspaceAccess"];
export type TeamspaceRole = Schemas["TeamspaceRole"];
export type TeamspaceMember = Schemas["TeamspaceMember"];

const path = (id: string, teamspaceId: string) => ({
  params: { path: { id, teamspace_id: teamspaceId } },
});

/**
 * The teamspaces the user sees, the default one first, and, for owners and
 * admins, the private ones they are not in, by name only.
 */
export function teamspacesQuery(workspaceId: string) {
  return queryOptions({
    queryKey: keys.teamspaces(workspaceId),
    queryFn: ({ signal }) =>
      call(
        api.GET("/api/v1/workspaces/{id}/teamspaces", {
          params: { path: { id: workspaceId } },
          signal,
        }),
      ),
    staleTime: 30 * 1000,
  });
}

export function teamspaceQuery(workspaceId: string, teamspaceId: string) {
  return queryOptions({
    queryKey: keys.teamspace(workspaceId, teamspaceId),
    queryFn: ({ signal }) =>
      call(
        api.GET("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}", {
          ...path(workspaceId, teamspaceId),
          signal,
        }),
      ),
  });
}

export function teamspaceMembersQuery(workspaceId: string, teamspaceId: string) {
  return queryOptions({
    queryKey: keys.teamspaceMembers(workspaceId, teamspaceId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}/members", {
            ...path(workspaceId, teamspaceId),
            signal,
          }),
        )
      ).members,
  });
}

/**
 * Reads the list and the one teamspace again, its members included. Not
 * awaited: a change can take its item off the page, and the callbacks of the
 * page's own call only run while that item is still there.
 */
function refreshTeamspace(queryClient: QueryClient, workspaceId: string, id: string) {
  void queryClient.invalidateQueries({ queryKey: keys.teamspaces(workspaceId) });
  void queryClient.invalidateQueries({ queryKey: keys.teamspace(workspaceId, id) });
}

/**
 * Joining or leaving a teamspace also changes what the user may do with its
 * pages, and which shared pages they reach through it rather than through
 * "Shared with you", so the trees and the pages read are read again too.
 */
function refreshMembership(queryClient: QueryClient, workspaceId: string, id: string) {
  refreshTeamspace(queryClient, workspaceId, id);
  void queryClient.invalidateQueries({ queryKey: keys.pageTrees(workspaceId) });
  void queryClient.invalidateQueries({ queryKey: keys.pages(workspaceId) });
}

/** Creates an open, closed or private teamspace, with the user as its owner. */
export function useCreateTeamspace(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateTeamspace"]) =>
      call(
        api.POST("/api/v1/workspaces/{id}/teamspaces", {
          params: { path: { id: workspaceId } },
          body,
        }),
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.teamspaces(workspaceId) }),
  });
}

export function useUpdateTeamspace(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: Schemas["UpdateTeamspace"] & { id: string }) =>
      call(
        api.PATCH("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}", {
          ...path(workspaceId, id),
          body,
        }),
      ),
    onSuccess: (teamspace) => refreshTeamspace(queryClient, workspaceId, teamspace.id),
  });
}

export function useDeleteTeamspace(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.DELETE("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}", path(workspaceId, id))),
    onSuccess: (_, id) => {
      queryClient.removeQueries({ queryKey: keys.teamspace(workspaceId, id) });
      void queryClient.invalidateQueries({ queryKey: keys.teamspaces(workspaceId) });
    },
  });
}

export function useJoinTeamspace(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      call(
        api.POST("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}/join", path(workspaceId, id)),
      ),
    onSuccess: (_, id) => refreshMembership(queryClient, workspaceId, id),
  });
}

export function useLeaveTeamspace(workspaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(
        api.POST("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}/leave", path(workspaceId, id)),
      ),
    onSuccess: (_, id) => refreshMembership(queryClient, workspaceId, id),
  });
}

/** Adds a member of the workspace who is not a guest, as a member unless told. */
export function useAddTeamspaceMember(workspaceId: string, id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["AddTeamspaceMember"]) =>
      call(
        api.POST("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}/members", {
          ...path(workspaceId, id),
          body,
        }),
      ),
    onSuccess: () => refreshTeamspace(queryClient, workspaceId, id),
  });
}

export function useChangeTeamspaceRole(workspaceId: string, id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: TeamspaceRole }) =>
      call(
        api.PATCH("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}/members/{user_id}", {
          params: { path: { id: workspaceId, teamspace_id: id, user_id: userId } },
          body: { role },
        }),
      ),
    onSuccess: () => refreshTeamspace(queryClient, workspaceId, id),
  });
}

export function useRemoveTeamspaceMember(workspaceId: string, id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) =>
      send(
        api.DELETE("/api/v1/workspaces/{id}/teamspaces/{teamspace_id}/members/{user_id}", {
          params: { path: { id: workspaceId, teamspace_id: id, user_id: userId } },
        }),
      ),
    onSuccess: () => refreshTeamspace(queryClient, workspaceId, id),
  });
}
