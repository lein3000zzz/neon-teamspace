import { queryOptions } from "@tanstack/react-query";
import { api } from "./client";
import { ApiError } from "./errors";

/** Instance name and server version. Public, and only changes on a deploy or a settings edit. */
export const metaQuery = queryOptions({
  queryKey: ["meta"],
  queryFn: async ({ signal }) => {
    const { data, error, response } = await api.GET("/api/v1/meta", { signal });
    if (data === undefined) {
      throw new ApiError(response.status, error);
    }
    return data;
  },
  staleTime: 5 * 60 * 1000,
});
