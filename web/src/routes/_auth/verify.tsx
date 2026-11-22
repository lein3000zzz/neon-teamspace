import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { VerifyPage } from "@/pages/auth/verify";

/** The link mailed to verify an address. */
export const Route = createFileRoute("/_auth/verify")({
  validateSearch: (search: Record<string, unknown>): { token?: string } => {
    const token = searchText(search["token"]);
    return token === undefined ? {} : { token };
  },
  component: VerifyPage,
});
