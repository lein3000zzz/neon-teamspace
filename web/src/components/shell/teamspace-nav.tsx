import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Compass, Lock } from "lucide-react";
import { useId } from "react";
import { teamspacesQuery } from "@/api/teamspaces";
import type { WorkspaceSummary } from "@/api/workspaces";
import { EntityMark } from "@/components/nodes/entity-mark";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";
import { navLink, navHeading } from "./nav-styles";

type TeamspaceNavProps = {
  workspace: WorkspaceSummary;
  onNavigate?: (() => void) | undefined;
};

/**
 * The teamspaces the user is in, each in its own hue, private ones marked,
 * then the way to the others. Not for guests, who are in none.
 */
export function TeamspaceNav({ workspace, onNavigate }: TeamspaceNavProps) {
  // The column and the open drawer are mounted together, so ids must be unique per copy.
  const headingId = useId();
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const joined = teamspaces.data?.teamspaces.filter((teamspace) => teamspace.role) ?? [];

  return (
    <div className="grid gap-1">
      <p id={headingId} className={navHeading}>
        {strings.nav.teamspaces}
      </p>
      {teamspaces.isError && (
        <div className="flex flex-wrap items-center gap-2 px-2.5 text-xs text-muted-foreground">
          {strings.nav.teamspacesFailed}
          <Button
            variant="ghost"
            size="sm"
            disabled={teamspaces.isFetching}
            onClick={() => void teamspaces.refetch()}
          >
            {strings.nav.retry}
          </Button>
        </div>
      )}
      <ul aria-labelledby={headingId} className="grid gap-0.5">
        {joined.map((teamspace) => (
          <li key={teamspace.id}>
            <Link
              to="/w/$slug/teamspaces/$teamspaceId"
              params={{ slug: workspace.slug, teamspaceId: teamspace.id }}
              onClick={onNavigate}
              {...(teamspace.access === "private"
                ? { "aria-label": strings.nav.privateTeamspace(teamspace.name) }
                : {})}
              className={navLink}
            >
              <EntityMark
                name={teamspace.name}
                icon={teamspace.icon}
                hue={teamspace.hue}
                size="sm"
              />
              <span className="min-w-0 flex-1 truncate">{teamspace.name}</span>
              {teamspace.access === "private" && <Lock aria-hidden className="size-3.5 shrink-0" />}
            </Link>
          </li>
        ))}
        <li>
          <Link
            to="/w/$slug/teamspaces"
            params={{ slug: workspace.slug }}
            activeOptions={{ exact: true }}
            onClick={onNavigate}
            className={navLink}
          >
            <Compass aria-hidden className="size-4" />
            {strings.nav.browseTeamspaces}
          </Link>
        </li>
      </ul>
    </div>
  );
}
