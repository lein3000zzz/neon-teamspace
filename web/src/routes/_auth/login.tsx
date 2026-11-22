import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { LoginPage } from "@/pages/auth/login";

export const Route = createFileRoute("/_auth/login")({
  validateSearch: (search: Record<string, unknown>): { next?: string } => {
    const next = searchText(search["next"]);
    return next === undefined ? {} : { next };
  },
  component: LoginPage,
});
