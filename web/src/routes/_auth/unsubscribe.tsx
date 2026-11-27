import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { UnsubscribePage } from "@/pages/auth/unsubscribe";

/** The link at the foot of a notification mail, which stops them without signing in. */
export const Route = createFileRoute("/_auth/unsubscribe")({
  validateSearch: (search: Record<string, unknown>): { token?: string } => {
    const token = searchText(search["token"]);
    return token === undefined ? {} : { token };
  },
  component: UnsubscribePage,
});
