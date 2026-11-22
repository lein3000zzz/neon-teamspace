import { createFileRoute } from "@tanstack/react-router";
import { UsersPage } from "@/pages/admin/users";

export const Route = createFileRoute("/_app/admin/")({
  component: UsersPage,
});
