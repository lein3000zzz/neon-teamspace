import { useQuery } from "@tanstack/react-query";
import { Link, Navigate } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useState } from "react";
import { workspacesQuery } from "@/api/workspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { PageSection } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { useMe } from "@/components/shell/signed-in";
import { Button } from "@/components/ui/button";
import { InvitationList } from "@/components/workspaces/invitation-list";
import { lastWorkspaceId, pickWorkspace } from "@/lib/last-workspace";
import { returnsToInvite } from "@/lib/links";
import { strings } from "@/lib/strings";

const copy = strings.workspaces.empty;

/**
 * Where the app opens: the workspace used last, while the user is still in
 * it, else their first. Someone in none gets the way into one: creating it,
 * where the instance lets them, or the invitations that wait for them. A tab
 * that opened a workspace invite link signed out goes back to it first, once
 * signed in or up.
 */
export function StartPage() {
  const me = useMe();
  const list = useQuery(workspacesQuery);
  // An invite link opened before signing in, or up, is where this tab was going.
  const [heldInvite] = useState(returnsToInvite);

  if (heldInvite) {
    return <Navigate to="/workspace-invite" replace />;
  }
  if (list.isPending) {
    return <Loading />;
  }
  if (list.isError) {
    return (
      <ErrorNotice
        error={list.error}
        retrying={list.isFetching}
        onRetry={() => void list.refetch()}
      />
    );
  }
  const target = pickWorkspace(list.data.workspaces, lastWorkspaceId(me.id));
  if (target !== undefined) {
    return <Navigate to="/w/$slug" params={{ slug: target.slug }} replace />;
  }
  return <NoWorkspace canCreate={list.data.can_create} />;
}

function NoWorkspace({ canCreate }: { canCreate: boolean }) {
  return (
    <div className="grid gap-10">
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede}>
        <div className="mt-3 grid max-w-[62ch] gap-3">
          {canCreate ? (
            <div className="flex flex-wrap items-center gap-3">
              <Button asChild>
                <Link to="/workspaces/new">
                  <Plus aria-hidden />
                  {copy.create}
                </Link>
              </Button>
              <span className="text-sm text-muted-foreground">{copy.createHint}</span>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{copy.adminsOnly}</p>
          )}
          <p className="text-sm text-muted-foreground">{copy.getInvited}</p>
        </div>
      </PageHeader>
      <PageSection title={copy.invitations}>
        <InvitationList />
      </PageSection>
    </div>
  );
}
