import { queryOptions, useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { hasCode } from "./errors";
import { keys } from "./keys";

export type WorkspaceSummary = Schemas["WorkspaceSummary"];
export type Workspace = Schemas["Workspace"];
export type WorkspaceList = Schemas["WorkspaceList"];
export type WorkspaceRole = Schemas["WorkspaceRole"];
export type Person = Schemas["Person"];
export type Member = Schemas["Member"];
export type SentInvitation = Schemas["SentInvitation"];
export type ReceivedInvitation = Schemas["ReceivedInvitation"];
export type InviteLink = Schemas["InviteLink"];
export type CreatedInviteLink = Schemas["CreatedInviteLink"];
export type InviteLinkPreview = Schemas["InviteLinkPreview"];

const path = (id: string) => ({ params: { path: { id } } });

/**
 * The workspaces the signed-in user is in, by name, and whether they may
 * create one. The sidebar and every workspace page read it, to find a
 * workspace by the slug in the address.
 */
export const workspacesQuery = queryOptions({
  queryKey: keys.workspaces,
  queryFn: ({ signal }) => call(api.GET("/api/v1/workspaces", { signal })),
  staleTime: 60 * 1000,
});

/** A workspace with its member count, limits and settings. */
export function workspaceQuery(id: string) {
  return queryOptions({
    queryKey: keys.workspaceDetail(id),
    queryFn: ({ signal }) => call(api.GET("/api/v1/workspaces/{id}", { ...path(id), signal })),
  });
}

/**
 * Reads one workspace's data again, and the list, which holds its name and the
 * role. Not awaited: a change can take its item off the page, and the
 * callbacks of the page's own call only run while that item is still there.
 */
function refreshWorkspace(queryClient: QueryClient, id: string) {
  void queryClient.invalidateQueries({ queryKey: keys.workspace(id) });
  void queryClient.invalidateQueries({ queryKey: keys.workspaces });
}

/**
 * Forgets a workspace the user is no longer in, left or deleted: out of the
 * list at once, so the page that moves on does not land in it again, then the
 * list is read again, unawaited, as the page moving on is the one to wait for.
 */
function dropWorkspace(queryClient: QueryClient, id: string) {
  queryClient.setQueryData<WorkspaceList>(
    keys.workspaces,
    (list) =>
      list && { ...list, workspaces: list.workspaces.filter((workspace) => workspace.id !== id) },
  );
  queryClient.removeQueries({ queryKey: keys.workspace(id) });
  void queryClient.invalidateQueries({ queryKey: keys.workspaces });
}

/**
 * Puts a workspace's new state in the list straight away, before the list is
 * read again, so a page that moves to its new address finds it there.
 */
function storeWorkspace(queryClient: QueryClient, workspace: Workspace) {
  const { id, name, slug, icon, hue, hue_override, role, created_at } = workspace;
  const summary: WorkspaceSummary = {
    id,
    name,
    slug,
    hue,
    role,
    created_at,
    ...(icon === undefined ? {} : { icon }),
    ...(hue_override === undefined ? {} : { hue_override }),
  };
  queryClient.setQueryData(keys.workspaceDetail(id), workspace);
  queryClient.setQueryData<WorkspaceList>(keys.workspaces, (list) => {
    if (list === undefined) {
      return list;
    }
    const others = list.workspaces.filter((entry) => entry.id !== id);
    return { ...list, workspaces: [...others, summary] };
  });
}

/** Makes a workspace with its General teamspace, the user owning both. */
export function useCreateWorkspace() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateWorkspace"]) =>
      call(api.POST("/api/v1/workspaces", { body })),
    onSuccess: async (workspace) => {
      storeWorkspace(queryClient, workspace);
      await queryClient.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}

/**
 * Changes the name, slug, icon, hue or settings, for owners and admins.
 * `onSaved` runs as soon as the server answers, before the list changes: a new
 * slug takes the page's address away, and the page moves to the new one first.
 */
export function useUpdateWorkspace(id: string, onSaved?: (workspace: Workspace) => void) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["UpdateWorkspace"]) =>
      call(api.PATCH("/api/v1/workspaces/{id}", { ...path(id), body })),
    onSuccess: async (workspace) => {
      onSaved?.(workspace);
      storeWorkspace(queryClient, workspace);
      await queryClient.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}

/**
 * Deletes the workspace with everything in it; its owners only, typing its
 * name. `onDeleted` runs before the workspace leaves the cache, so the page in
 * it moves on first, as for useLeaveWorkspace.
 */
export function useDeleteWorkspace(id: string, onDeleted: () => void) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (confirmName: string) =>
      send(
        api.DELETE("/api/v1/workspaces/{id}", { ...path(id), body: { confirm_name: confirmName } }),
      ),
    onSuccess: () => {
      onDeleted();
      dropWorkspace(queryClient, id);
    },
  });
}

/** Everyone in the workspace, owners first; a guest gets only themselves. */
export function membersQuery(id: string) {
  return queryOptions({
    queryKey: keys.members(id),
    queryFn: async ({ signal }) =>
      (await call(api.GET("/api/v1/workspaces/{id}/members", { ...path(id), signal }))).members,
  });
}

export function useChangeMemberRole(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: WorkspaceRole }) =>
      call(
        api.PATCH("/api/v1/workspaces/{id}/members/{user_id}", {
          params: { path: { id, user_id: userId } },
          body: { role },
        }),
      ),
    onSuccess: () => refreshWorkspace(queryClient, id),
  });
}

export function useRemoveMember(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) =>
      send(
        api.DELETE("/api/v1/workspaces/{id}/members/{user_id}", {
          params: { path: { id, user_id: userId } },
        }),
      ),
    onSuccess: () => refreshWorkspace(queryClient, id),
  });
}

/**
 * Leaves the workspace. `onLeft` runs before it leaves the cache: the page in
 * it would go with it, and its own callbacks with the page, so it moves on first.
 */
export function useLeaveWorkspace(id: string, onLeft: () => void) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => send(api.POST("/api/v1/workspaces/{id}/leave", path(id))),
    onSuccess: () => {
      onLeft();
      dropWorkspace(queryClient, id);
    },
  });
}

/** Makes another member an owner, and the owner asking an admin. */
export function useTransferOwnership(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) =>
      send(
        api.POST("/api/v1/workspaces/{id}/transfer", { ...path(id), body: { user_id: userId } }),
      ),
    onSuccess: () => refreshWorkspace(queryClient, id),
  });
}

/** The invitations the workspace sent that wait for their answer. */
export function sentInvitationsQuery(id: string) {
  return queryOptions({
    queryKey: keys.sentInvitations(id),
    queryFn: async ({ signal }) =>
      (await call(api.GET("/api/v1/workspaces/{id}/invitations", { ...path(id), signal })))
        .invitations,
  });
}

/** Invites an account by its address; it answers from its pending invitations. */
export function useInvite(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateInvitation"]) =>
      call(api.POST("/api/v1/workspaces/{id}/invitations", { ...path(id), body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.sentInvitations(id) }),
  });
}

/** Takes an invitation back; it leaves the list, so the list is read again unawaited. */
export function useRevokeInvitation(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (invitationId: string) =>
      send(
        api.POST("/api/v1/workspaces/{id}/invitations/{invitation_id}/revoke", {
          params: { path: { id, invitation_id: invitationId } },
        }),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.sentInvitations(id) });
    },
  });
}

/** Every invite link of the workspace, newest first, without their tokens. */
export function inviteLinksQuery(id: string) {
  return queryOptions({
    queryKey: keys.inviteLinks(id),
    queryFn: async ({ signal }) =>
      (await call(api.GET("/api/v1/workspaces/{id}/invite-links", { ...path(id), signal }))).links,
  });
}

/** Makes an invite link. Its token is in this answer, and later only from its sealed copy. */
export function useCreateInviteLink(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateInviteLink"]) =>
      call(api.POST("/api/v1/workspaces/{id}/invite-links", { ...path(id), body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.inviteLinks(id) }),
  });
}

/** An invite link's token again, from its sealed copy. */
export function useInviteLinkToken(id: string) {
  return useMutation({
    mutationFn: async (linkId: string) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/invite-links/{link_id}/token", {
            params: { path: { id, link_id: linkId } },
          }),
        )
      ).token,
  });
}

export function useRevokeInviteLink(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (linkId: string) =>
      send(
        api.POST("/api/v1/workspaces/{id}/invite-links/{link_id}/revoke", {
          params: { path: { id, link_id: linkId } },
        }),
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.inviteLinks(id) }),
  });
}

/**
 * What an invite link admits to, without using it. A mutation rather than a
 * query, so the token leaves memory with the page that asked.
 */
export function usePreviewInviteLink() {
  return useMutation({
    mutationFn: (token: string) =>
      call(api.POST("/api/v1/workspace-invite-links/preview", { body: { token } })),
  });
}

/**
 * Joins the workspace of an invite link: using it is accepting it. Joining
 * ends any invitation into the same workspace that waited for the user, so
 * their invitations are read again too, unawaited.
 */
export function useJoinWithLink() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (token: string) =>
      call(api.POST("/api/v1/workspace-invite-links/join", { body: { token } })),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.myInvitations });
      return queryClient.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}

/** The invitations into workspaces that wait for the user's answer, newest first. */
export const myInvitationsQuery = queryOptions({
  queryKey: keys.myInvitations,
  queryFn: async ({ signal }) =>
    (await call(api.GET("/api/v1/account/invitations", { signal }))).invitations,
  staleTime: 60 * 1000,
});

/**
 * An invitation that answers not_found has ended since the list was read:
 * answered in another tab, revoked, past its end, or settled by joining the
 * workspace another way. The list is read again, unawaited, so it goes.
 */
function forgetEnded(queryClient: QueryClient, error: unknown) {
  if (hasCode(error, "not_found", "already_member")) {
    void queryClient.invalidateQueries({ queryKey: keys.myInvitations });
  }
}

/**
 * Joins the workspace with the invitation's role. The invitation leaves the
 * list, so the lists are read again unawaited, as for refreshWorkspace.
 */
export function useAcceptInvitation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => call(api.POST("/api/v1/account/invitations/{id}/accept", path(id))),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.myInvitations });
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
    },
    onError: (error) => forgetEnded(queryClient, error),
  });
}

export function useDeclineInvitation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      send(api.POST("/api/v1/account/invitations/{id}/decline", path(id))),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.myInvitations });
    },
    onError: (error) => forgetEnded(queryClient, error),
  });
}
