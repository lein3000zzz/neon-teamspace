import { createFileRoute } from "@tanstack/react-router";
import { searchText } from "@/lib/links";
import { RegisterPage } from "@/pages/auth/register";

/** An invite link is /register?invite=CODE; the page takes the code out of the address. */
export const Route = createFileRoute("/_auth/register")({
  validateSearch: (search: Record<string, unknown>): { invite?: string } => {
    const invite = searchText(search["invite"]);
    return invite === undefined ? {} : { invite };
  },
  component: RegisterPage,
});
