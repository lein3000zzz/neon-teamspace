import { createFileRoute } from "@tanstack/react-router";
import { TokensPage } from "@/pages/account/tokens";

export const Route = createFileRoute("/_app/account/tokens")({
  component: TokensPage,
});
