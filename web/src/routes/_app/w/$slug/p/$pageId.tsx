import { createFileRoute } from "@tanstack/react-router";
import { PageView } from "@/pages/pages/page";

/** A page by its id; the server answers not found for one the user may not see. */
export const Route = createFileRoute("/_app/w/$slug/p/$pageId")({
  component: PageView,
});
