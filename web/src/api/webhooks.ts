import { queryOptions, useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Webhook = Schemas["Webhook"];
export type WebhookEvent = Schemas["WebhookEvent"];
export type WebhookWithSecret = Schemas["WebhookWithSecret"];
export type DeliveryAttempt = Schemas["DeliveryAttempt"];
export type IncomingWebhook = Schemas["IncomingWebhook"];
export type CreatedIncomingWebhook = Schemas["CreatedIncomingWebhook"];
export type IncomingRequest = Schemas["IncomingRequest"];

/** Every event an outgoing webhook may ask for, in the order forms list them. */
export const WEBHOOK_EVENTS = [
  "row.assigned",
  "row.status_changed",
  "page.mentioned",
  "comment.replied",
] as const satisfies readonly WebhookEvent[];

const one = (id: string, webhookId: string) => ({
  params: { path: { id, webhook_id: webhookId } },
});
const hook = (id: string, hookId: string) => ({ params: { path: { id, hook_id: hookId } } });

/**
 * The workspace's outgoing webhooks the user may manage: every one for its
 * owners and admins, their own and their agents' for everyone else.
 */
export function webhooksQuery(id: string) {
  return queryOptions({
    queryKey: keys.webhooks(id),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/webhooks", { params: { path: { id } }, signal }),
        )
      ).webhooks,
  });
}

function refreshWebhooks(queryClient: QueryClient, id: string) {
  void queryClient.invalidateQueries({ queryKey: keys.webhooks(id) });
}

/** Registers a webhook; its secret is in this answer only. */
export function useCreateWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateWebhook"]) =>
      call(api.POST("/api/v1/workspaces/{id}/webhooks", { params: { path: { id } }, body })),
    onSuccess: () => refreshWebhooks(queryClient, id),
  });
}

/** Changes a webhook's URL or events, or turns it on or off. */
export function useUpdateWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ webhookId, body }: { webhookId: string; body: Schemas["UpdateWebhook"] }) =>
      call(
        api.PATCH("/api/v1/workspaces/{id}/webhooks/{webhook_id}", {
          ...one(id, webhookId),
          body,
        }),
      ),
    onSuccess: () => refreshWebhooks(queryClient, id),
  });
}

export function useDeleteWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (webhookId: string) =>
      send(api.DELETE("/api/v1/workspaces/{id}/webhooks/{webhook_id}", one(id, webhookId))),
    onSuccess: () => refreshWebhooks(queryClient, id),
  });
}

/** A new secret, in this answer only; the one before signs beside it for a day. */
export function useRotateWebhookSecret(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (webhookId: string) =>
      call(api.POST("/api/v1/workspaces/{id}/webhooks/{webhook_id}/secret", one(id, webhookId))),
    onSuccess: () => refreshWebhooks(queryClient, id),
  });
}

/** Sends a test event now, and answers how it went; the log keeps it. */
export function useTestWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (webhookId: string) =>
      call(api.POST("/api/v1/workspaces/{id}/webhooks/{webhook_id}/test", one(id, webhookId))),
    onSuccess: (_, webhookId) =>
      queryClient.invalidateQueries({ queryKey: keys.webhookDeliveries(id, webhookId) }),
  });
}

/** A webhook's last attempts, newest first. */
export function deliveriesQuery(id: string, webhookId: string) {
  return queryOptions({
    queryKey: keys.webhookDeliveries(id, webhookId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/webhooks/{webhook_id}/deliveries", {
            ...one(id, webhookId),
            signal,
          }),
        )
      ).attempts,
  });
}

/** The workspace's incoming webhooks, for its owners and admins. */
export function incomingWebhooksQuery(id: string) {
  return queryOptions({
    queryKey: keys.incomingWebhooks(id),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/incoming-webhooks", {
            params: { path: { id } },
            signal,
          }),
        )
      ).hooks,
  });
}

/** Reads the incoming webhooks again, and the agents, among which their integrations are. */
function refreshIncoming(queryClient: QueryClient, id: string) {
  void queryClient.invalidateQueries({ queryKey: keys.incomingWebhooks(id) });
  void queryClient.invalidateQueries({ queryKey: keys.agents(id) });
  void queryClient.invalidateQueries({ queryKey: keys.members(id) });
}

/** Makes an incoming webhook; its URL, and its secret, are in this answer only. */
export function useCreateIncomingWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Schemas["CreateIncomingWebhook"]) =>
      call(
        api.POST("/api/v1/workspaces/{id}/incoming-webhooks", { params: { path: { id } }, body }),
      ),
    onSuccess: () => refreshIncoming(queryClient, id),
  });
}

/** Renames an incoming webhook, or turns it on or off. */
export function useUpdateIncomingWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ hookId, body }: { hookId: string; body: Schemas["UpdateIncomingWebhook"] }) =>
      call(
        api.PATCH("/api/v1/workspaces/{id}/incoming-webhooks/{hook_id}", {
          ...hook(id, hookId),
          body,
        }),
      ),
    onSuccess: () => refreshIncoming(queryClient, id),
  });
}

export function useDeleteIncomingWebhook(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (hookId: string) =>
      send(api.DELETE("/api/v1/workspaces/{id}/incoming-webhooks/{hook_id}", hook(id, hookId))),
    onSuccess: () => refreshIncoming(queryClient, id),
  });
}

/** A new URL, in this answer only; the old one stops working at once. */
export function useRotateIncomingUrl(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (hookId: string) =>
      call(api.POST("/api/v1/workspaces/{id}/incoming-webhooks/{hook_id}/token", hook(id, hookId))),
    onSuccess: () => refreshIncoming(queryClient, id),
  });
}

/** A new signing secret, in this answer only; requests must be signed from now. */
export function useRotateIncomingSecret(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (hookId: string) =>
      call(
        api.POST("/api/v1/workspaces/{id}/incoming-webhooks/{hook_id}/secret", hook(id, hookId)),
      ),
    onSuccess: () => refreshIncoming(queryClient, id),
  });
}

/** Stops checking signatures: unsigned requests are taken again. */
export function useRemoveIncomingSecret(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (hookId: string) =>
      call(
        api.DELETE("/api/v1/workspaces/{id}/incoming-webhooks/{hook_id}/secret", hook(id, hookId)),
      ),
    onSuccess: () => refreshIncoming(queryClient, id),
  });
}

/** An incoming webhook's last requests, newest first. */
export function incomingRequestsQuery(id: string, hookId: string) {
  return queryOptions({
    queryKey: keys.incomingRequests(id, hookId),
    queryFn: async ({ signal }) =>
      (
        await call(
          api.GET("/api/v1/workspaces/{id}/incoming-webhooks/{hook_id}/requests", {
            ...hook(id, hookId),
            signal,
          }),
        )
      ).requests,
  });
}
