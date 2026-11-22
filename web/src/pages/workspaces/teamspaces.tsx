import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Lock, Plus } from "lucide-react";
import { useState } from "react";
import { teamspacesQuery, type Teamspace } from "@/api/teamspaces";
import { workspaceQuery } from "@/api/workspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { EntityMark } from "@/components/nodes/entity-mark";
import { ItemActions, ItemCard } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { TeamspaceEntry } from "@/components/workspaces/teamspace-entry";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { mayCreateTeamspace, seesTeamspaces } from "@/lib/workspace-rules";
import { NotAllowedPage } from "@/pages/not-allowed";

const copy = strings.teamspaces.browse;

/**
 * Every teamspace the user sees in the workspace: those they are in, to open,
 * and the others, to join or to ask to be added to.
 */
export function BrowseTeamspacesPage() {
  const workspace = useWorkspace();
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const details = useQuery(workspaceQuery(workspace.id));
  const [joined, setJoined] = useState<string>();
  const canCreate =
    details.data !== undefined &&
    mayCreateTeamspace(workspace.role, details.data.members_can_create_teamspaces);

  if (!seesTeamspaces(workspace.role)) {
    return <NotAllowedPage />;
  }
  return (
    <>
      <PageHeader eyebrow={workspace.name} title={copy.title} lede={copy.lede}>
        {canCreate && (
          <div className="mt-3">
            <Button asChild variant="outline">
              <Link to="/w/$slug/settings/teamspaces" params={{ slug: workspace.slug }}>
                <Plus aria-hidden />
                {copy.create}
              </Link>
            </Button>
          </div>
        )}
      </PageHeader>
      <div className="grid gap-4">
        <output className="block text-sm text-muted-foreground empty:hidden">
          {joined === undefined ? "" : copy.joined(joined)}
        </output>
        {teamspaces.isPending && <Loading />}
        {teamspaces.isError && (
          <ErrorNotice
            error={teamspaces.error}
            retrying={teamspaces.isFetching}
            onRetry={() => void teamspaces.refetch()}
          />
        )}
        {teamspaces.data && (
          <ul className="grid gap-3">
            {teamspaces.data.teamspaces.map((teamspace) => (
              <TeamspaceCard key={teamspace.id} teamspace={teamspace} onJoined={setJoined} />
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

function TeamspaceCard({
  teamspace,
  onJoined,
}: {
  teamspace: Teamspace;
  onJoined: (name: string) => void;
}) {
  const workspace = useWorkspace();
  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        <p className="flex min-w-0 items-center gap-2.5 font-medium">
          <EntityMark name={teamspace.name} icon={teamspace.icon} hue={teamspace.hue} />
          <span className="break-words">{teamspace.name}</span>
          {teamspace.access === "private" && (
            <Lock aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
          )}
        </p>
        <ChipList>
          <Chip tone={teamspace.role ? "strong" : "default"} hue={teamspace.hue}>
            {teamspace.role ? copy.in : strings.teamspaces.notIn}
          </Chip>
          <Chip>{strings.access[teamspace.access]}</Chip>
          <Chip>{strings.teamspaces.count(teamspace.member_count)}</Chip>
        </ChipList>
        <p className="text-sm text-muted-foreground">{strings.accessHints[teamspace.access]}</p>
        <TeamspaceEntry teamspace={teamspace} onJoined={onJoined} />
      </div>
      <ItemActions>
        <Button asChild variant="outline" size="sm">
          <Link
            to="/w/$slug/teamspaces/$teamspaceId"
            params={{ slug: workspace.slug, teamspaceId: teamspace.id }}
            aria-label={copy.openLabel(teamspace.name)}
          >
            {copy.open}
          </Link>
        </Button>
      </ItemActions>
    </ItemCard>
  );
}
