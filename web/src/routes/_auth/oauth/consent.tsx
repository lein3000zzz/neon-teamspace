import { createFileRoute, redirect } from "@tanstack/react-router";
import { meQuery } from "@/api/account";
import { AUTHORIZATION_PARAMS, type AuthorizationParams } from "@/api/oauth";
import { searchText } from "@/lib/links";
import { ConsentPage } from "@/pages/auth/oauth-consent";

/** The request the authorization endpoint sent the browser here with, or its problem. */
export type ConsentSearch = AuthorizationParams & { error?: string };

/**
 * The consent page an OAuth client's authorization request ends on. Someone
 * signed out signs in first and comes back here, request and all; a request
 * that did not hold shows its problem to anyone.
 */
export const Route = createFileRoute("/_auth/oauth/consent")({
  validateSearch: (search: Record<string, unknown>): ConsentSearch => {
    const read: ConsentSearch = {};
    for (const key of [...AUTHORIZATION_PARAMS, "error"] as const) {
      const value = searchText(search[key]);
      if (value !== undefined) {
        read[key] = value;
      }
    }
    return read;
  },
  beforeLoad: async ({ context, location, search }) => {
    if (search.error !== undefined) {
      return;
    }
    const me = await context.queryClient.ensureQueryData(meQuery);
    if (me === null) {
      throw redirect({ to: "/login", search: { next: location.href }, replace: true });
    }
  },
  component: ConsentPage,
});
