import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Compass, Lock, Settings, UserPlus } from "lucide-react";
import { teamspacesQuery } from "@/api/teamspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { EntityMark } from "@/components/nodes/entity-mark";
import { PageSection } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { manages, seesTeamspaces } from "@/lib/workspace-rules";

const copy = strings.workspaces.home;

/** A workspace's first page, until it has pages: its teamspaces, and the ways onward. */
export function WorkspaceHomePage() {
  const workspace = useWorkspace();
  const guest = !seesTeamspaces(workspace.role);
  return (
    <div className="grid gap-10">
      <PageHeader
        eyebrow={
          <span className="flex items-center gap-2">
            <EntityMark name={workspace.name} icon={workspace.icon} hue={workspace.hue} size="sm" />
            {workspace.name}
          </span>
        }
        title={copy.title}
        lede={guest ? copy.guestLede : copy.lede}
      >
        <div className="mt-3 flex flex-wrap gap-2">
          {manages(workspace.role) && (
            <Button asChild>
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
      {!guest && <YourTeamspaces />}
    </div>
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
