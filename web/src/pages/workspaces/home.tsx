import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Compass, Lock, Plus, Settings, Upload, UserPlus } from "lucide-react";
import { useState } from "react";
import { favoritesQuery, sharedPagesQuery } from "@/api/pages";
import { teamspacesQuery } from "@/api/teamspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { EntityMark } from "@/components/nodes/entity-mark";
import { ImportDialog } from "@/components/pages/import-dialog";
import { PageCards } from "@/components/pages/page-cards";
import { useNewPage } from "@/components/pages/use-new-page";
import { PageSection } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { hasPrivatePages } from "@/lib/page-rules";
import { forestOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { manages, seesTeamspaces } from "@/lib/workspace-rules";

const copy = strings.workspaces.home;

/**
 * A workspace's first page: the ways to a new page and onward, the user's
 * favorites, their teamspaces, and the pages shared with them, which are all
 * a guest has.
 */
export function WorkspaceHomePage() {
  const workspace = useWorkspace();
  const guest = !seesTeamspaces(workspace.role);
  const newPage = useNewPage(workspace);
  const [importing, setImporting] = useState(false);
  return (
    <div className="grid gap-10">
      <PageHeader
        eyebrow={
          <span className="flex items-center gap-2">
            <EntityMark name={workspace.name} icon={workspace.icon} hue={workspace.hue} size="sm" />
            {copy.eyebrow}
          </span>
        }
        title={workspace.name}
        lede={guest ? copy.guestLede : copy.lede}
      >
        <div className="mt-3 flex flex-wrap gap-2">
          {hasPrivatePages(workspace.role) && (
            <>
              <Button disabled={newPage.pending} onClick={() => newPage.start({ private: true })}>
                <Plus aria-hidden />
                {newPage.pending ? strings.pages.creating : copy.newPage}
              </Button>
              <Button variant="outline" onClick={() => setImporting(true)}>
                <Upload aria-hidden />
                {copy.importPrivate}
              </Button>
            </>
          )}
          {manages(workspace.role) && (
            <Button asChild variant="outline">
              <Link to="/w/$slug/settings/members" params={{ slug: workspace.slug }}>
                <UserPlus aria-hidden />
                {copy.invite}
              </Link>
            </Button>
          )}
          {!guest && (
            <Button asChild variant="outline">
              <Link to="/w/$slug/teamspaces" params={{ slug: workspace.slug }}>
                <Compass aria-hidden />
                {copy.browse}
              </Link>
            </Button>
          )}
          <Button asChild variant="outline">
            <Link to="/w/$slug/settings" params={{ slug: workspace.slug }}>
              <Settings aria-hidden />
              {copy.settings}
            </Link>
          </Button>
        </div>
      </PageHeader>
      {importing && (
        <ImportDialog
          workspace={workspace}
          place={{ private: true }}
          placeName={strings.pages.transfer.privatePlace}
          open
          onOpenChange={setImporting}
        />
      )}
      <Favorites />
      {!guest && <YourTeamspaces />}
      <SharedWithYou guest={guest} />
    </div>
  );
}

function Favorites() {
  const workspace = useWorkspace();
  const favorites = useQuery(favoritesQuery(workspace.id));
  if (favorites.data === undefined || favorites.data.length === 0) {
    return null;
  }
  return (
    <PageSection title={copy.favorites} hint={copy.favoritesHint}>
      <PageCards workspace={workspace} pages={favorites.data} />
    </PageSection>
  );
}

function SharedWithYou({ guest }: { guest: boolean }) {
  const workspace = useWorkspace();
  const shared = useQuery(sharedPagesQuery(workspace.id));
  const top = shared.data === undefined ? [] : forestOf(shared.data).roots;
  if (!guest && top.length === 0) {
    return null;
  }
  return (
    <PageSection title={copy.shared} hint={copy.sharedHint}>
      {shared.isPending && <Loading />}
      {shared.isError && (
        <ErrorNotice
          error={shared.error}
          retrying={shared.isFetching}
          onRetry={() => void shared.refetch()}
        />
      )}
      {shared.isSuccess && top.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.noneShared}</p>
      )}
      {top.length > 0 && <PageCards workspace={workspace} pages={top} />}
    </PageSection>
  );
}

function YourTeamspaces() {
  const workspace = useWorkspace();
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const joined = teamspaces.data?.teamspaces.filter((teamspace) => teamspace.role) ?? [];
  return (
    <PageSection title={copy.teamspaces} hint={copy.teamspacesHint}>
      {teamspaces.isPending && <Loading />}
      {teamspaces.isError && (
        <ErrorNotice
          error={teamspaces.error}
          retrying={teamspaces.isFetching}
          onRetry={() => void teamspaces.refetch()}
        />
      )}
      {joined.length > 0 && (
        <ul className="grid gap-2 sm:grid-cols-2">
          {joined.map((teamspace) => (
            <li key={teamspace.id}>
              <Link
                to="/w/$slug/teamspaces/$teamspaceId"
                params={{ slug: workspace.slug, teamspaceId: teamspace.id }}
                className="flex min-h-(--tap-lg) items-center gap-3 rounded-lg border border-border bg-card px-3 py-2 text-sm transition-colors hover:border-primary"
              >
                <EntityMark name={teamspace.name} icon={teamspace.icon} hue={teamspace.hue} />
                <span className="grid min-w-0 flex-1">
                  <span className="truncate font-medium">{teamspace.name}</span>
                  <span className="text-xs text-muted-foreground">
                    {strings.access[teamspace.access]} ·{" "}
                    {strings.teamspaces.count(teamspace.member_count)}
                  </span>
                </span>
                {teamspace.access === "private" && (
                  <Lock aria-hidden className="size-3.5 text-muted-foreground" />
                )}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </PageSection>
  );
}
