import { queryOptions } from "@tanstack/react-query";
import { api } from "./client";
import { withDeadline } from "./deadline";
import { ApiError } from "./errors";
import { keys } from "./keys";

/** How long to wait for the server before reporting it unreachable. */
export const META_TIMEOUT_MS = 10_000;

/** Instance name and server version. Public, and only changes on a deploy or a settings edit. */
export const metaQuery = queryOptions({
  queryKey: keys.meta,
  queryFn: ({ signal }) =>
    withDeadline(signal, META_TIMEOUT_MS, async (deadline) => {
      const { data, error, response } = await api.GET("/api/v1/meta", { signal: deadline });
      if (data === undefined) {
        throw ApiError.from(response, error);
      }
      return data;
    }),
  staleTime: 5 * 60 * 1000,
});
