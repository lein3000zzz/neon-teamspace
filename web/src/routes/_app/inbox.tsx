import { createFileRoute } from "@tanstack/react-router";
import { InboxPage } from "@/pages/inbox/inbox";

/** The user's inbox, from every workspace they are in. */
export const Route = createFileRoute("/_app/inbox")({
  component: InboxPage,
});
