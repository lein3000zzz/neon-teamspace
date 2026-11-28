import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { call } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type Updates = Schemas["Updates"];
export type Release = Schemas["Release"];
export type UpdaterStatus = Schemas["UpdaterStatus"];

/**
 * The server's version, the newest release its last check saw on GitHub, and the updater,
 * which the server asks for at most 3 seconds. The server checks once a day, so what it
 * knows changes seldom; the panel reads it afresh when it opens.
 */
export const updatesQuery = queryOptions({
  queryKey: keys.updates,
  queryFn: ({ signal }) => call(api.GET("/api/v1/admin/updates", { signal })),
});

/** Asks GitHub for the releases now, while the release check is on. */
export function useCheckUpdates() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => call(api.POST("/api/v1/admin/updates/check")),
    onSuccess: (updates) => queryClient.setQueryData(keys.updates, updates),
  });
}

/**
 * Has the updater update the stack to a release, which answers once the updater has
 * started; updateProgressQuery follows it from there.
 */
export function useApplyUpdate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (version: string) =>
      call(api.POST("/api/v1/admin/updates/apply", { body: { version } })),
    onSuccess: (updates) => queryClient.setQueryData(keys.updates, updates),
  });
}

/** How often an update's progress is asked. */
export const PROGRESS_EVERY_MS = 2_000;

/** How long the server may take to come back on the new release before the page gives up. */
export const PROGRESS_FOR_MS = 10 * 60 * 1000;

/**
 * Where an update stands, as the page sees it:
 *
 * - `updating`: the old server answers, and with it the updater's status, when it could ask;
 * - `restarting`: nothing answers, as the server or the web origin restarts;
 * - `failed`: the updater says this update failed, and the stack runs what it ran before;
 * - `done`: the server runs the new release, and the updater is no longer at work.
 */
export type Progress =
  | { phase: "updating"; status: UpdaterStatus | undefined }
  | { phase: "restarting" }
  | { phase: "failed"; status: UpdaterStatus }
  | { phase: "done" };

/**
 * How the update to `version` goes. Every failure to answer is a step of the update rather
 * than an error: the server and the web origin restart in the middle of it.
 */
export function updateProgressQuery(version: string) {
  return queryOptions({
    queryKey: keys.updateProgress(version),
    queryFn: async ({ signal }): Promise<Progress> => {
      const [meta, updates] = await Promise.all([
        call(api.GET("/api/v1/meta", { signal, cache: "no-store" })).catch(() => undefined),
        call(api.GET("/api/v1/admin/updates", { signal })).catch(() => undefined),
      ]);
      const status = updates?.updater.status ?? undefined;
      const ours = status?.version === version;
      if (meta?.version === version && !(ours && status?.state === "updating")) {
        return { phase: "done" };
      }
      if (ours && status?.state === "failed") {
        return { phase: "failed", status };
      }
      if (meta === undefined && updates === undefined) {
        return { phase: "restarting" };
      }
      return { phase: "updating", status: ours ? status : undefined };
    },
    retry: false,
    gcTime: 0,
  });
}
