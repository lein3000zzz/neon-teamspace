import { useNavigate } from "@tanstack/react-router";
import { createContext, use, useCallback, useMemo, type ReactNode } from "react";
import type { Database, Property } from "@/api/databases";
import { useWorkspacePeople } from "@/components/people/use-people";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { WIDE_QUERY, useMediaQuery } from "@/lib/media";
import type { Someone } from "@/lib/people";
import { edits } from "@/lib/page-rules";
import { seesTeamspaces } from "@/lib/workspace-rules";

type Scope = {
  database: Database;
  /** Whether the user changes the schema and the views and adds rows: they edit the database. */
  editsDatabase: boolean;
  /** Whether they may see and set people, which guests may not. */
  seesPeople: boolean;
  /**
   * The workspace's members by id, agents among them, and its agents deleted
   * since, to name the people values hold.
   */
  people: ReadonlyMap<string, Someone>;
  /** Says what went wrong with a change made in the database. */
  report: (error: unknown) => void;
  /** Shown inside another page: smaller, and without the page's own header. */
  compact: boolean;
  /** Opens the view's filter with a new condition on a property, where there is one. */
  startFilter: (property: Property) => void;
};

const ScopeContext = createContext<Scope | null>(null);

/** The database the components inside show, and what they may do with it. */
export function useDatabaseScope(): Scope {
  const scope = use(ScopeContext);
  if (scope === null) {
    throw new Error("useDatabaseScope must be used inside a DatabaseScope");
  }
  return scope;
}

type DatabaseScopeProps = {
  database: Database;
  report: (error: unknown) => void;
  compact?: boolean;
  startFilter?: (property: Property) => void;
  children: ReactNode;
};

const noFilter = () => {};

/**
 * What every view, cell and editor of a database needs to know: the database,
 * whether the user edits it, who the people its values name are, and where to
 * say a change failed. Nothing in a database in the trash changes.
 */
export function DatabaseScope({
  database,
  report,
  compact = false,
  startFilter = noFilter,
  children,
}: DatabaseScopeProps) {
  const workspace = useWorkspace();
  const seesPeople = seesTeamspaces(workspace.role);
  const people = useWorkspacePeople(workspace.id, seesPeople);
  const trashed = (database.page.trashed_at ?? null) !== null;
  const value = useMemo(
    () => ({
      database,
      editsDatabase: edits(database.page.access) && !trashed,
      seesPeople,
      people,
      report,
      compact,
      startFilter,
    }),
    [database, trashed, seesPeople, people, report, compact, startFilter],
  );
  return <ScopeContext value={value}>{children}</ScopeContext>;
}

/**
 * Opens a row: beside the database, as a peek, on a wide screen, where both
 * fit; in place of it, as a full page, on a narrower one. The peek's entry
 * says it was opened from the database, for its closing to step back to it.
 */
export function useOpenRow() {
  const workspace = useWorkspace();
  const navigate = useNavigate({ from: "/w/$slug/p/$pageId" });
  const wide = useMediaQuery(WIDE_QUERY);
  return useCallback(
    (rowId: string, full = false) => {
      if (wide && !full) {
        void navigate({
          search: (previous) => ({ ...previous, peek: rowId }),
          state: { peeked: true },
        });
      } else {
        void navigate({
          to: "/w/$slug/p/$pageId",
          params: { slug: workspace.slug, pageId: rowId },
          search: {},
        });
      }
    },
    [navigate, wide, workspace.slug],
  );
}
