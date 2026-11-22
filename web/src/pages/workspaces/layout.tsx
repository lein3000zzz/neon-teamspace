import { useQuery } from "@tanstack/react-query";
import { Link, Outlet, useParams } from "@tanstack/react-router";
import { useEffect } from "react";
import { workspacesQuery } from "@/api/workspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { PageHeader } from "@/components/shell/page-header";
import { useMe } from "@/components/shell/signed-in";
import { Button } from "@/components/ui/button";
import { WorkspaceContext } from "@/components/workspaces/workspace-context";
import { rememberWorkspace } from "@/lib/last-workspace";
import { strings } from "@/lib/strings";

/**
 * Every page under /w/{slug}: the slug is looked up in the user's own list of
 * workspaces, so a workspace they are not in, or no longer, gets the same
 * not-found page as one that does not exist. The workspace found is the one
 * the app opens on next time.
 */
export function WorkspaceLayout() {
  const { slug } = useParams({ from: "/_app/w/$slug" });
  const me = useMe();
  const list = useQuery(workspacesQuery);
  const workspace = list.data?.workspaces.find((candidate) => candidate.slug === slug);

  useEffect(() => {
    if (workspace !== undefined) {
      rememberWorkspace(me.id, workspace.id);
    }
  }, [me.id, workspace]);

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
  if (workspace === undefined) {
    return <WorkspaceNotFound slug={slug} />;
  }
  return (
    <WorkspaceContext value={workspace}>
      <Outlet />
    </WorkspaceContext>
  );
}

function WorkspaceNotFound({ slug }: { slug: string }) {
  const copy = strings.workspaces.notFound;
  return (
    <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede(slug)}>
      <div className="mt-3">
        <Button asChild variant="outline">
          <Link to="/">{copy.home}</Link>
        </Button>
      </div>
    </PageHeader>
  );
}
