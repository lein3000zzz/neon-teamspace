import { Link } from "@tanstack/react-router";
import { Check, ChevronsUpDown, Plus, Settings } from "lucide-react";
import type { WorkspaceSummary } from "@/api/workspaces";
import { EntityMark } from "@/components/nodes/entity-mark";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useCurrentWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";

const copy = strings.workspaces.switcher;

type WorkspaceSwitcherProps = {
  /** Called after a link is followed, so the drawer can close itself. */
  onNavigate?: (() => void) | undefined;
};

/**
 * The workspace the sidebar is about, and a menu of every other one the user
 * is in, each with its mark and the user's role there; then the workspace's
 * settings, and creating one when the instance lets them.
 */
export function WorkspaceSwitcher({ onNavigate }: WorkspaceSwitcherProps) {
  const { list, current } = useCurrentWorkspace();

  if (list.isPending) {
    return <p className="px-2.5 text-xs text-muted-foreground">{copy.loading}</p>;
  }
  if (list.isError) {
    return (
      <div className="flex flex-wrap items-center gap-2 px-2.5 text-xs text-muted-foreground">
        {copy.failed}
        <Button
          variant="ghost"
          size="sm"
          disabled={list.isFetching}
          onClick={() => void list.refetch()}
        >
          {strings.nav.retry}
        </Button>
      </div>
    );
  }
  const { workspaces, can_create: canCreate } = list.data;
  if (workspaces.length === 0) {
    return (
      <Link
        to="/"
        onClick={onNavigate}
        className="flex min-h-(--tap) items-center gap-2.5 rounded-md border border-dashed border-border px-2.5 text-sm text-muted-foreground hover:text-foreground"
      >
        <Plus aria-hidden className="size-4" />
        {copy.none}
      </Link>
    );
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={
          current === undefined
            ? copy.menu
            : copy.label(current.name, strings.roles[current.role].toLowerCase())
        }
        className="flex min-h-(--tap) w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-1 text-left transition-colors hover:bg-accent aria-expanded:bg-accent"
      >
        {current === undefined ? (
          <span className="flex-1 text-sm text-muted-foreground">{copy.menu}</span>
        ) : (
          <WorkspaceLine workspace={current} />
        )}
        <ChevronsUpDown aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{copy.menu}</DropdownMenuLabel>
          {workspaces.map((workspace) => {
            const isCurrent = workspace.id === current?.id;
            return (
              <DropdownMenuItem key={workspace.id} asChild>
                <Link
                  to="/w/$slug"
                  params={{ slug: workspace.slug }}
                  onClick={onNavigate}
                  aria-label={copy.entry(workspace.name, strings.roles[workspace.role], isCurrent)}
                >
                  <WorkspaceLine workspace={workspace} />
                  {isCurrent && <Check aria-hidden className="text-primary" />}
                </Link>
              </DropdownMenuItem>
            );
          })}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        {current !== undefined && (
          <DropdownMenuItem asChild>
            <Link to="/w/$slug/settings" params={{ slug: current.slug }} onClick={onNavigate}>
              <Settings aria-hidden />
              {copy.settings}
            </Link>
          </DropdownMenuItem>
        )}
        {canCreate && (
          <DropdownMenuItem asChild>
            <Link to="/workspaces/new" onClick={onNavigate}>
              <Plus aria-hidden />
              {copy.create}
            </Link>
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** A workspace's mark, its name, and the user's role in it. */
function WorkspaceLine({ workspace }: { workspace: WorkspaceSummary }) {
  return (
    <span className="flex min-w-0 flex-1 items-center gap-2.5">
      <EntityMark name={workspace.name} icon={workspace.icon} hue={workspace.hue} />
      <span className="grid min-w-0 leading-tight">
        <span className="truncate text-sm font-medium text-foreground">{workspace.name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {strings.roles[workspace.role]}
        </span>
      </span>
    </span>
  );
}
