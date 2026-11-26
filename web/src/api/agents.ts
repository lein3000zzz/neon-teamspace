import { queryOptions, useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Agent = Schemas["Agent"];
export type AgentTeamspace = Schemas["AgentTeamspace"];
export type AgentStatus = Schemas["AgentStatus"];
export type CreatedApiToken = Schemas["CreatedApiToken"];

const path = (id: string, agentId: string) => ({ params: { path: { id, agent_id: agentId } } });

/**
 * A workspace's agents and integrations, deleted ones too, which what they
 * made still names. Everyone but guests reads them; a guest gets none.
 */
export function agentsQuery(id: string) {
  return queryOptions({
    queryKey: keys.agents(id),
    queryFn: async ({ signal }) =>
      (await call(api.GET("/api/v1/workspaces/{id}/agents", { params: { path: { id } }, signal })))
        .agents,
  });
}

/**
 * Reads the agents again, and what shows them: the members, among whom they
 * are, and the workspace, which counts them. Not awaited, as for the
 * workspace's own changes.
 */
function refreshAgents(queryClient: QueryClient, id: string) {
  void queryClient.invalidateQueries({ queryKey: keys.agents(id) });
  void queryClient.invalidateQueries({ queryKey: keys.members(id) });
  void queryClient.invalidateQueries({ queryKey: keys.workspaceDetail(id) });
}

/** Makes an agent the user owns. */
export function useCreateAgent(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateAgent"]) =>
      call(api.POST("/api/v1/workspaces/{id}/agents", { params: { path: { id } }, body })),
    onSuccess: () => refreshAgents(queryClient, id),
  });
}

/** Renames or recolours an agent, for its owner. */
export function useUpdateAgent(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, body }: { agentId: string; body: Schemas["UpdateAgent"] }) =>
      call(api.PATCH("/api/v1/workspaces/{id}/agents/{agent_id}", { ...path(id, agentId), body })),
    onSuccess: () => refreshAgents(queryClient, id),
  });
}

/** Hands an agent to another person, for owners and admins; its tokens are revoked. */
export function useTransferAgent(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, ownerId }: { agentId: string; ownerId: string }) =>
      call(
        api.POST("/api/v1/workspaces/{id}/agents/{agent_id}/transfer", {
          ...path(id, agentId),
          body: { owner_id: ownerId },
        }),
      ),
    onSuccess: (_, { agentId }) => {
      refreshAgents(queryClient, id);
      void queryClient.invalidateQueries({ queryKey: keys.agentTokens(id, agentId) });
    },
  });
}

/** Deletes an agent: it leaves the workspace, and its tokens are revoked. */
export function useDeleteAgent(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (agentId: string) =>
      send(api.DELETE("/api/v1/workspaces/{id}/agents/{agent_id}", path(id, agentId))),
    onSuccess: () => refreshAgents(queryClient, id),
  });
}

/**
 * The teamspaces an agent is in, of those the user sees, never a private one
 * they are not in, with whether they may take it out of each.
 */
export function agentTeamspacesQuery(id: string, agentId: string) {
  return queryOptions({
    queryKey: keys.agentTeamspaces(id, agentId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/agents/{agent_id}/teamspaces", {
            ...path(id, agentId),
            signal,
          }),
        )
      ).teamspaces,
  });
}

/** An agent's tokens, without their secrets. */
export function agentTokensQuery(id: string, agentId: string) {
  return queryOptions({
    queryKey: keys.agentTokens(id, agentId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/agents/{agent_id}/tokens", {
            ...path(id, agentId),
            signal,
          }),
        )
      ).tokens,
  });
}

/** Makes a token that acts as the agent. Its secret is in this answer only, and never again. */
export function useCreateAgentToken(id: string, agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateAgentToken"]) =>
      call(
        api.POST("/api/v1/workspaces/{id}/agents/{agent_id}/tokens", {
          ...path(id, agentId),
          body,
        }),
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.agentTokens(id, agentId) }),
  });
}

export function useRevokeAgentToken(id: string, agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (tokenId: string) =>
      send(
        api.DELETE("/api/v1/workspaces/{id}/agents/{agent_id}/tokens/{token_id}", {
          params: { path: { id, agent_id: agentId, token_id: tokenId } },
        }),
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.agentTokens(id, agentId) }),
  });
}
