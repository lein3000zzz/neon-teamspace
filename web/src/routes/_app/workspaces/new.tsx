import { createFileRoute } from "@tanstack/react-router";
import { CreateWorkspacePage } from "@/pages/workspaces/create";

export const Route = createFileRoute("/_app/workspaces/new")({
  component: CreateWorkspacePage,
});
