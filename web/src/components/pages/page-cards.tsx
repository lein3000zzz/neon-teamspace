import { Link } from "@tanstack/react-router";
import type { PageKind } from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { titleOf } from "@/lib/page-tree";
import { PageGlyph } from "./page-tree";

type Listed = { id: string; title: string; icon?: string | null | undefined; kind?: PageKind };

type PageCardsProps = {
  workspace: WorkspaceSummary;
  pages: readonly Listed[];
  label?: string;
};

/** Pages as cards in a grid, each the way to open it, for the workspace's and a teamspace's page. */
export function PageCards({ workspace, pages, label }: PageCardsProps) {
  return (
    <ul
      {...(label === undefined ? {} : { "aria-label": label })}
      className="grid gap-2 sm:grid-cols-2"
    >
      {pages.map((page) => (
        <li key={page.id}>
          <Link
            to="/w/$slug/p/$pageId"
            params={{ slug: workspace.slug, pageId: page.id }}
            className="flex min-h-(--tap-lg) items-center gap-3 rounded-lg border border-border bg-card px-3 py-2 text-sm transition-colors hover:border-primary"
          >
            <PageGlyph icon={page.icon} kind={page.kind} />
            <span className="min-w-0 flex-1 truncate font-medium">{titleOf(page)}</span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
