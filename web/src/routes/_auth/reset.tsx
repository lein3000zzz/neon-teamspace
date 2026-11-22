import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { ResetPage } from "@/pages/auth/reset";

/** The link mailed to set a new password. */
export const Route = createFileRoute("/_auth/reset")({
  validateSearch: (search: Record<string, unknown>): { token?: string } => {
    const token = searchText(search["token"]);
    return token === undefined ? {} : { token };
  },
  component: ResetPage,
});
