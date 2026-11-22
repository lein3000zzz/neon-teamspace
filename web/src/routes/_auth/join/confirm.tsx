import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { JoinConfirmPage } from "@/pages/auth/join-confirm";

/** The link mailed to confirm a request to join. */
export const Route = createFileRoute("/_auth/join/confirm")({
  validateSearch: (search: Record<string, unknown>): { token?: string } => {
    const token = searchText(search["token"]);
    return token === undefined ? {} : { token };
  },
  component: JoinConfirmPage,
});
