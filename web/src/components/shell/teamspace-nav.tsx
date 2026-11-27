import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Compass, Lock, Plus } from "lucide-react";
import { useId } from "react";
import { teamspacePagesQuery } from "@/api/pages";
import { teamspacesQuery, type Teamspace } from "@/api/teamspaces";
import type { WorkspaceSummary } from "@/api/workspaces";
import { EntityMark } from "@/components/nodes/entity-mark";
import { usePageNav } from "@/components/pages/page-nav-state";
import {
  Disclosure,
  PageTree,
  treeLink,
  treeRow,
  useSpaceDrop,
} from "@/components/pages/page-tree";
import { useNewPage } from "@/components/pages/use-new-page";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";
import { navHeading, navLink } from "./nav-styles";

type TeamspaceNavProps = {
  workspace: WorkspaceSummary;
  onNavigate?: (() => void) | undefined;
};

/**
 * The teamspaces the user is in, each in its own hue, private ones marked,
 * each with its tree of pages, open until folded; then the way to the others.
 * Not for guests, who are in none.
 */
export function TeamspaceNav({ workspace, onNavigate }: TeamspaceNavProps) {
  // The column and the open drawer are mounted together, so ids must be unique per copy.
  const headingId = useId();
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const joined = teamspaces.data?.teamspaces.filter((teamspace) => teamspace.role) ?? [];

  return (
    <div className="grid grid-cols-1 gap-1">
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
      <ul aria-labelledby={headingId} className="grid grid-cols-1 gap-0.5">
        {joined.map((teamspace) => (
          <TeamspaceSection
            key={teamspace.id}
            workspace={workspace}
            teamspace={teamspace}
            onNavigate={onNavigate}
          />
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

type TeamspaceSectionProps = {
  workspace: WorkspaceSummary;
  teamspace: Teamspace;
  onNavigate?: (() => void) | undefined;
};

/** A teamspace's row, which also takes a page dropped on it, and its pages under it. */
function TeamspaceSection({ workspace, teamspace, onNavigate }: TeamspaceSectionProps) {
  const nav = usePageNav();
  const space = { kind: "teamspace", id: teamspace.id, name: teamspace.name } as const;
  const drop = useSpaceDrop(workspace, space);
  const newPage = useNewPage(workspace, onNavigate);
  const open = !nav.isFolded(teamspace.id);
  return (
    <li>
      <div
        data-row=""
        {...(drop.over ? { "data-drop": "inside" } : {})}
        {...drop.handlers}
        className={treeRow}
      >
        <Disclosure
          expandable
          open={open}
          title={teamspace.name}
          onToggle={() => nav.setFolded(teamspace.id, open)}
        />
        <Link
          to="/w/$slug/teamspaces/$teamspaceId"
          params={{ slug: workspace.slug, teamspaceId: teamspace.id }}
          onClick={onNavigate}
          {...(teamspace.access === "private"
            ? { "aria-label": strings.nav.privateTeamspace(teamspace.name) }
            : {})}
          className={treeLink}
        >
          <EntityMark name={teamspace.name} icon={teamspace.icon} hue={teamspace.hue} size="sm" />
          <span className="min-w-0 flex-1 truncate">{teamspace.name}</span>
          {teamspace.access === "private" && <Lock aria-hidden className="size-3.5 shrink-0" />}
        </Link>
        <div data-row-actions="over" className="flex shrink-0 items-center">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={strings.pages.tree.addTo(teamspace.name)}
            disabled={newPage.pending}
            onClick={() => newPage.start({ teamspace_id: teamspace.id })}
          >
            <Plus aria-hidden />
          </Button>
        </div>
      </div>
      {open && (
        <TeamspacePages workspace={workspace} teamspace={teamspace} onNavigate={onNavigate} />
      )}
    </li>
  );
}

function TeamspacePages({ workspace, teamspace, onNavigate }: TeamspaceSectionProps) {
  const pages = useQuery(teamspacePagesQuery(workspace.id, teamspace.id));
  if (pages.isError) {
    return <TreeFailed onRetry={() => void pages.refetch()} retrying={pages.isFetching} />;
  }
  if (pages.data === undefined) {
    return null;
  }
  if (pages.data.length === 0) {
    return <p className="py-1 pl-9 text-xs text-muted-foreground">{strings.pages.tree.none}</p>;
  }
  return (
    <PageTree
      workspace={workspace}
      space={{ kind: "teamspace", id: teamspace.id, name: teamspace.name }}
      pages={pages.data}
      label={strings.pages.tree.pagesOf(teamspace.name)}
      onNavigate={onNavigate}
      depth={1}
    />
  );
}

/** A tree that did not load, and the way to ask again. */
export function TreeFailed({ onRetry, retrying }: { onRetry: () => void; retrying: boolean }) {
  return (
    <div className="flex flex-wrap items-center gap-2 py-1 pl-9 text-xs text-muted-foreground">
      {strings.pages.tree.failed}
      <Button variant="ghost" size="sm" disabled={retrying} onClick={onRetry}>
        {strings.pages.tree.retry}
      </Button>
    </div>
  );
}
