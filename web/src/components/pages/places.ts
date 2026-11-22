import { useQueries, useQuery } from "@tanstack/react-query";
import {
  privatePagesQuery,
  sharedPagesQuery,
  teamspacePagesQuery,
  type PagePlace,
  type TreePage,
} from "@/api/pages";
import { teamspacesQuery } from "@/api/teamspaces";
import type { WorkspaceSummary } from "@/api/workspaces";
import { hasPrivatePages } from "@/lib/page-rules";
import { forestOf, type PageForest } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { seesTeamspaces } from "@/lib/workspace-rules";

/** Where a tree of pages comes from, which says where a page at its top lives. */
export type Space =
  { kind: "teamspace"; id: string; name: string } | { kind: "private" } | { kind: "shared" };

/**
 * Where a page goes when placed beside one at the top of a tree: the top of
 * its teamspace, or of the private pages. A page shared on its own has a
 * parent the user does not see, so nothing goes beside it.
 */
export function topOf(space: Space): PagePlace | undefined {
  switch (space.kind) {
    case "teamspace":
      return { teamspace_id: space.id };
    case "private":
      return { private: true };
    case "shared":
      return undefined;
  }
}

/** Where a page goes when placed beside `page` in a tree of `space`. */
export function besideOf(space: Space, page: TreePage, forest: PageForest): PagePlace | undefined {
  const parent = page.parent_id ?? undefined;
  if (parent !== undefined && forest.byId.has(parent)) {
    return { parent_id: parent };
  }
  return topOf(space);
}

export type Place = { space: Space; forest: PageForest };

/**
 * Every tree of pages the user has in a workspace: each teamspace they are in,
 * their private pages, and the pages shared with them. Read only while
 * `enabled`, as when the dialog that needs them is open.
 */
export function usePlaces(workspace: WorkspaceSummary, enabled: boolean): Place[] {
  const member = seesTeamspaces(workspace.role);
  const teamspaces = useQuery({ ...teamspacesQuery(workspace.id), enabled: enabled && member });
  const joined = teamspaces.data?.teamspaces.filter((teamspace) => teamspace.role) ?? [];
  const trees = useQueries({
    queries: joined.map((teamspace) => ({
      ...teamspacePagesQuery(workspace.id, teamspace.id),
      enabled,
    })),
  });
  const own = useQuery({
    ...privatePagesQuery(workspace.id),
    enabled: enabled && hasPrivatePages(workspace.role),
  });
  const shared = useQuery({ ...sharedPagesQuery(workspace.id), enabled });

  const places: Place[] = joined.map((teamspace, index) => ({
    space: { kind: "teamspace", id: teamspace.id, name: teamspace.name },
    forest: forestOf(trees[index]?.data ?? []),
  }));
  if (hasPrivatePages(workspace.role)) {
    places.push({ space: { kind: "private" }, forest: forestOf(own.data ?? []) });
  }
  places.push({ space: { kind: "shared" }, forest: forestOf(shared.data ?? []) });
  return places;
}

/** What a space is called beside a page in it. */
export function spaceName(space: Space): string {
  switch (space.kind) {
    case "teamspace":
      return space.name;
    case "private":
      return strings.pages.move.private;
    case "shared":
      return strings.nav.shared;
  }
}
