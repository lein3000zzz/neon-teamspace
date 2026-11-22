import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Plus, X } from "lucide-react";
import { useId } from "react";
import { favoritesQuery, privatePagesQuery, sharedPagesQuery } from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { PageMenu } from "@/components/pages/page-menu";
import { usePageNav } from "@/components/pages/page-nav-state";
import { PageGlyph, PageTree, treeLink, treeRow, useSpaceDrop } from "@/components/pages/page-tree";
import { useNewPage } from "@/components/pages/use-new-page";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { navHeading } from "./nav-styles";
import { TreeFailed } from "./teamspace-nav";

type SectionProps = {
  workspace: WorkspaceSummary;
  onNavigate?: (() => void) | undefined;
};

/** The pages the user marked, in their order, while there are any. */
export function FavoritesNav({ workspace, onNavigate }: SectionProps) {
  const headingId = useId();
  const favorites = useQuery(favoritesQuery(workspace.id));
  if (favorites.data === undefined || favorites.data.length === 0) {
    return null;
  }
  return (
    <div className="grid gap-1">
      <p id={headingId} className={navHeading}>
        {strings.nav.favorites}
      </p>
      <ul aria-labelledby={headingId} className="grid gap-px">
        {favorites.data.map((page) => (
          <li key={page.id}>
            <div data-row="" className={treeRow}>
              <Link
                to="/w/$slug/p/$pageId"
                params={{ slug: workspace.slug, pageId: page.id }}
                onClick={onNavigate}
                className={`${treeLink} pl-2.5`}
              >
                <PageGlyph icon={page.icon} />
                <span className="min-w-0 flex-1 truncate">{titleOf(page)}</span>
              </Link>
              <div data-row-actions="" className="flex shrink-0 items-center">
                <PageMenu workspace={workspace} page={page} />
              </div>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * The member's own pages in the workspace, which nobody else sees; its
 * heading takes a page dropped on it, to the top of them. Not for guests.
 */
export function PrivateNav({ workspace, onNavigate }: SectionProps) {
  const headingId = useId();
  const pages = useQuery(privatePagesQuery(workspace.id));
  const drop = useSpaceDrop(workspace, { kind: "private" });
  const newPage = useNewPage(workspace, onNavigate);
  return (
    <div className="grid gap-1">
      <div
        data-row=""
        {...(drop.over ? { "data-drop": "inside" } : {})}
        {...drop.handlers}
        className="flex items-center justify-between gap-2 rounded-md data-[drop=inside]:bg-neon-soft"
      >
        <p id={headingId} className={navHeading}>
          {strings.nav.private}
        </p>
        <div data-row-actions="" className="flex shrink-0 items-center">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={strings.pages.tree.addPrivate}
            disabled={newPage.pending}
            onClick={() => newPage.start({ private: true })}
            className="text-muted-foreground"
          >
            <Plus aria-hidden />
          </Button>
        </div>
      </div>
      {pages.isError && (
        <TreeFailed onRetry={() => void pages.refetch()} retrying={pages.isFetching} />
      )}
      {pages.data?.length === 0 && (
        <p className="px-2.5 text-xs text-muted-foreground">{strings.pages.tree.noPrivate}</p>
      )}
      {pages.data !== undefined && pages.data.length > 0 && (
        <PageTree
          workspace={workspace}
          space={{ kind: "private" }}
          pages={pages.data}
          label={strings.nav.private}
          onNavigate={onNavigate}
        />
      )}
    </div>
  );
}

/**
 * The pages shared with the user from places they do not otherwise reach,
 * with the pages under them, while there are any. For a guest, all they see.
 */
export function SharedNav({ workspace, onNavigate }: SectionProps) {
  const headingId = useId();
  const pages = useQuery(sharedPagesQuery(workspace.id));
  if (pages.isError) {
    return <TreeFailed onRetry={() => void pages.refetch()} retrying={pages.isFetching} />;
  }
  if (pages.data === undefined || pages.data.length === 0) {
    return null;
  }
  return (
    <div className="grid gap-1">
      <p id={headingId} className={navHeading}>
        {strings.nav.shared}
      </p>
      <PageTree
        workspace={workspace}
        space={{ kind: "shared" }}
        pages={pages.data}
        label={strings.nav.shared}
        onNavigate={onNavigate}
      />
    </div>
  );
}

/** What the last change from the sidebar did, or why it failed, until dismissed. */
export function PageNavNotice() {
  const nav = usePageNav();
  const notice = nav.notice;
  if (notice === null) {
    return null;
  }
  const dismiss = (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={strings.pages.tree.dismiss}
      onClick={() => nav.setNotice(null)}
      className="-my-1 justify-self-end"
    >
      <X aria-hidden />
    </Button>
  );
  if (notice.tone === "error") {
    return <ErrorNotice error={notice.error}>{dismiss}</ErrorNotice>;
  }
  return (
    <Alert live="polite">
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-foreground">{notice.text}</p>
        {notice.undo && (
          <Button variant="outline" size="sm" onClick={notice.undo}>
            {strings.pages.menu.undo}
          </Button>
        )}
        {dismiss}
      </div>
    </Alert>
  );
}
