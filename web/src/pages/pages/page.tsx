import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { ImagePlus, Maximize2, Smile, Star, UserPlus } from "lucide-react";
import { lazy, Suspense, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { databaseQuery } from "@/api/databases";
import { hasCode } from "@/api/errors";
import { keys } from "@/api/keys";
import {
  breadcrumbsQuery,
  favoritesQuery,
  pageQuery,
  useAddFavorite,
  useDeletePage,
  useRemoveFavorite,
  useRestorePage,
  useUpdatePage,
  type Page,
  type UpdatePage,
} from "@/api/pages";
import { teamspacesQuery } from "@/api/teamspaces";
import { ConnectionClosed, ConnectionIndicator } from "@/components/editor/connection-indicator";
import { STARTING, type Connection } from "@/components/editor/connection-state";
import { DatabaseView } from "@/components/databases/database-view";
import { RowProperties } from "@/components/databases/row-properties";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { CoverBand, CoverPicker } from "@/components/pages/cover-picker";
import { IconPicker } from "@/components/pages/icon-picker";
import { PageMenu } from "@/components/pages/page-menu";
import { PersonLabel } from "@/components/people/person-label";
import { useWorkspacePeople } from "@/components/people/use-people";
import { usePageNav } from "@/components/pages/page-nav-state";
import { documentTitle, PageTitle } from "@/components/pages/page-title";
import { ShareDialog } from "@/components/pages/share-dialog";
import { StyleMenu } from "@/components/pages/style-menu";
import { PageHeader } from "@/components/shell/page-header";
import { useMe } from "@/components/shell/signed-in";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { edits, mayShare } from "@/lib/page-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { seesTeamspaces } from "@/lib/workspace-rules";

const PageEditor = lazy(() => import("@/components/editor/page-editor"));

const copy = strings.pages.page;

/**
 * What a page's address says beside the page: the view a database shows, and
 * the row shown beside it, a peek, when one is open.
 */
export type PageSearch = { view?: string; peek?: string };

const ROUTE = "/_app/w/$slug/p/$pageId";

/**
 * A page: the way down to it, its cover, icon and title, and its content in
 * the live editor; a database's views, for a database; a row's properties
 * above its content, for a row. A page the user may not see answers not
 * found, as one that does not exist would; one in the trash shows only to its
 * editors, with the way back out. A row opened beside the page shows in a
 * panel over its side.
 */
export function PageView() {
  const { pageId } = useParams({ from: ROUTE });
  const { peek } = useSearch({ from: ROUTE });
  return (
    <>
      <PageById key={pageId} pageId={pageId} />
      {peek !== undefined && <RowPeek rowId={peek} />}
    </>
  );
}

function PageById({ pageId }: { pageId: string }) {
  const workspace = useWorkspace();
  const page = useQuery(pageQuery(workspace.id, pageId));

  if (page.isPending) {
    return <Loading />;
  }
  if (page.isError) {
    if (hasCode(page.error, "not_found")) {
      return <PageNotFound />;
    }
    return (
      <ErrorNotice
        error={page.error}
        retrying={page.isFetching}
        onRetry={() => void page.refetch()}
      />
    );
  }
  if (page.data.workspace_id !== workspace.id) {
    return <PageNotFound />;
  }
  return <PageScreen key={page.data.id} page={page.data} />;
}

type PageScreenProps = {
  page: Page;
  /** Shown beside a database, as a peek: the tab keeps its title, and the page offers its own. */
  peek?: boolean;
};

function PageScreen({ page, peek = false }: PageScreenProps) {
  const workspace = useWorkspace();
  const me = useMe();
  const queryClient = useQueryClient();
  const update = useUpdatePage(workspace.id, page.id);
  const [connection, setConnection] = useState<Connection>(STARTING);
  // Bumped to open the live connection afresh, after the server closed it.
  const [generation, setGeneration] = useState(0);
  const focusContent = useRef<(() => void) | null>(null);
  const trashed = (page.trashed_at ?? null) !== null;
  const editable = edits(page.access) && !trashed;
  const set = (body: UpdatePage) => update.mutate(body);

  useEffect(() => {
    if (peek) {
      return;
    }
    const before = document.title;
    document.title = documentTitle(page);
    return () => {
      document.title = before;
    };
  }, [page, peek]);

  const onAccessChanged = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: keys.page(workspace.id, page.id) });
  }, [queryClient, workspace.id, page.id]);
  const reconnect = () => setGeneration((count) => count + 1);

  return (
    <article
      data-page-font={page.style.font}
      {...(page.style.small_text ? { "data-small-text": "" } : {})}
      {...(page.style.full_width || page.kind === "database" ? { "data-page-width": "full" } : {})}
      // One column as wide as the page, whatever is inside: a database's views scroll
      // sideways within it rather than widen it past a phone's screen.
      className="grid grid-cols-1 gap-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <Breadcrumbs page={page} />
        <div className="flex flex-wrap items-center gap-1">
          {peek && (
            <Button asChild variant="ghost" size="sm">
              <Link to="/w/$slug/p/$pageId" params={{ slug: workspace.slug, pageId: page.id }}>
                <Maximize2 aria-hidden />
                {strings.databases.peek.openFull}
              </Link>
            </Button>
          )}
          {page.kind !== "database" && (
            <ConnectionIndicator
              connection={connection}
              readOnly={!editable || connection.access === "view"}
            />
          )}
          {!trashed && <FavoriteToggle page={page} />}
          {!trashed && mayShare(workspace.role, page.access) && <ShareButton page={page} />}
          {editable && (
            <StyleMenu
              style={page.style}
              disabled={update.isPending}
              widthChoice={page.kind !== "database"}
              onChange={(style) => set({ style })}
            />
          )}
          {!trashed && <PageMenu workspace={workspace} page={page} />}
        </div>
      </div>

      {trashed && <TrashedBanner page={page} />}
      {update.isError && <ErrorNotice error={update.error} />}
      <ConnectionClosed connection={connection} onReconnect={reconnect} />

      <header className="grid gap-3">
        {page.cover && (
          <CoverBand cover={page.cover}>
            {editable && (
              <CoverPicker cover={page.cover} onPick={(cover) => set({ cover })}>
                <Button variant="outline" size="sm" className="bg-background">
                  {copy.changeCover}
                </Button>
              </CoverPicker>
            )}
          </CoverBand>
        )}
        <div className="grid gap-2" style={{ paddingInline: "var(--page-inset)" }}>
          {page.icon && (
            <PageIcon page={page} editable={editable} onPick={(icon) => set({ icon })} />
          )}
          {editable && (!page.icon || !page.cover) && (
            <div className="flex flex-wrap gap-1">
              {!page.icon && (
                <IconPicker icon={page.icon} onPick={(icon) => set({ icon })}>
                  <Button variant="ghost" size="sm">
                    <Smile aria-hidden />
                    {copy.addIcon}
                  </Button>
                </IconPicker>
              )}
              {!page.cover && (
                <CoverPicker cover={page.cover} onPick={(cover) => set({ cover })}>
                  <Button variant="ghost" size="sm">
                    <ImagePlus aria-hidden />
                    {copy.addCover}
                  </Button>
                </CoverPicker>
              )}
            </div>
          )}
        </div>
        <PageTitle
          workspaceId={workspace.id}
          page={page}
          editable={editable}
          onEnter={() => focusContent.current?.()}
        />
        {!editable && !trashed && (
          <p
            className="text-sm text-muted-foreground"
            style={{ paddingInline: "var(--page-inset)" }}
          >
            {copy.readOnly}
          </p>
        )}
        {page.kind !== "row" && <Byline page={page} />}
      </header>

      {page.kind === "row" && <RowProperties page={page} />}
      {page.kind === "database" ? (
        <DatabaseBody page={page} />
      ) : (
        <Suspense fallback={<Loading label={strings.pages.editor.loading} />}>
          {/* Access changes in place: the same document goes on, read-only or not. */}
          <PageEditor
            key={`${page.id}:${generation}`}
            pageId={page.id}
            editable={editable}
            holdsDatabases={page.kind === "page"}
            user={{ id: me.id, name: me.display_name, hue: me.hue }}
            onConnection={setConnection}
            onAccessChanged={onAccessChanged}
            focusRef={focusContent}
          />
        </Suspense>
      )}
    </article>
  );
}

/**
 * Who made the page and who changed it last, an agent with the person it acts
 * for. Guests, who know no other member, are told neither.
 */
function Byline({ page }: { page: Page }) {
  const workspace = useWorkspace();
  const people = useWorkspacePeople(workspace.id, seesTeamspaces(workspace.role));
  const made = page.created_by ?? null;
  const changed = page.last_edited_by ?? null;
  if (made === null && changed === null) {
    return null;
  }
  return (
    <section aria-label={copy.byline} style={{ paddingInline: "var(--page-inset)" }}>
      <dl className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {made !== null && (
          <div className="flex min-w-0 items-center gap-1.5">
            <dt>{copy.createdBy}</dt>
            <dd className="min-w-0">
              <PersonLabel person={people.get(made)} />
            </dd>
          </div>
        )}
        {changed !== null && changed !== made && (
          <div className="flex min-w-0 items-center gap-1.5">
            <dt>{copy.editedBy}</dt>
            <dd className="min-w-0">
              <PersonLabel person={people.get(changed)} />
            </dd>
          </div>
        )}
      </dl>
    </section>
  );
}

/**
 * A database's page below its title: its views, the one the address names in
 * view, or its first. Its page's title and icon are the page's as they are
 * typed, before the database is read again.
 */
function DatabaseBody({ page }: { page: Page }) {
  const workspace = useWorkspace();
  const { view } = useSearch({ from: ROUTE });
  const navigate = useNavigate({ from: "/w/$slug/p/$pageId" });
  const database = useQuery(databaseQuery(workspace.id, page.id));
  if (database.isPending) {
    return <Loading label={strings.databases.loading} />;
  }
  if (database.isError) {
    return (
      <ErrorNotice
        error={database.error}
        retrying={database.isFetching}
        onRetry={() => void database.refetch()}
      />
    );
  }
  return (
    <div style={{ paddingLeft: "var(--page-inset)" }}>
      <DatabaseView
        database={{ ...database.data, page }}
        viewId={view}
        onViewChange={(next) =>
          void navigate({ search: (previous) => ({ ...previous, view: next }), replace: true })
        }
      />
    </div>
  );
}

/**
 * A row beside the page that shows its database, in a panel over the page's
 * side: the row's page as it is on its own, with the way to open it so.
 * Closing it, or Escape, goes back to the database where it was.
 */
function RowPeek({ rowId }: { rowId: string }) {
  const workspace = useWorkspace();
  const navigate = useNavigate({ from: "/w/$slug/p/$pageId" });
  const page = useQuery(pageQuery(workspace.id, rowId));
  const close = () =>
    void navigate({
      search: (previous) => {
        const next = { ...previous };
        delete next.peek;
        return next;
      },
    });
  const title = page.data === undefined ? strings.pages.untitled : titleOf(page.data);
  return (
    <Sheet open onOpenChange={(open) => !open && close()}>
      <SheetContent side="right" closeLabel={strings.databases.peek.close}>
        <SheetTitle className="sr-only">{strings.databases.peek.label(title)}</SheetTitle>
        <div className="mt-10 pr-3">
          {page.isPending && <Loading />}
          {page.isError &&
            (hasCode(page.error, "not_found") ? (
              <p className="text-sm text-muted-foreground">{copy.notFoundLede}</p>
            ) : (
              <ErrorNotice
                error={page.error}
                retrying={page.isFetching}
                onRetry={() => void page.refetch()}
              />
            ))}
          {page.data !== undefined && <PageScreen key={page.data.id} page={page.data} peek />}
        </div>
      </SheetContent>
    </Sheet>
  );
}

type PageIconProps = { page: Page; editable: boolean; onPick: (icon: string | null) => void };

function PageIcon({ page, editable, onPick }: PageIconProps) {
  const glyph = (
    <span aria-hidden className="text-5xl leading-none">
      {page.icon}
    </span>
  );
  if (!editable) {
    return <div className="justify-self-start">{glyph}</div>;
  }
  return (
    <IconPicker icon={page.icon} onPick={onPick}>
      <button
        type="button"
        aria-label={copy.changeIcon}
        className="grid size-16 place-items-center justify-self-start rounded-lg hover:bg-accent"
      >
        {glyph}
      </button>
    </IconPicker>
  );
}

/**
 * The way down to the page: its space, when the user sees it, then each page
 * above it they see, from the highest. A guest's starts at the page shared
 * with them.
 */
function Breadcrumbs({ page }: { page: Page }) {
  const workspace = useWorkspace();
  const me = useMe();
  const nav = usePageNav();
  const crumbs = useQuery(breadcrumbsQuery(workspace.id, page.id));
  const teamspaces = useQuery({
    ...teamspacesQuery(workspace.id),
    enabled: workspace.role !== "guest",
  });
  const teamspace = teamspaces.data?.teamspaces.find((entry) => entry.id === page.teamspace_id);
  const { openAll, setFolded } = nav;

  // The page shows in the sidebar's tree, the pages above it opened.
  useEffect(() => {
    if (crumbs.data !== undefined) {
      openAll(crumbs.data.slice(0, -1).map((crumb) => crumb.id));
    }
    if (page.teamspace_id) {
      setFolded(page.teamspace_id, false);
    }
  }, [crumbs.data, page.teamspace_id, openAll, setFolded]);

  const above = crumbs.data?.slice(0, -1) ?? [];
  return (
    <nav aria-label={copy.breadcrumbs} className="min-w-0">
      <ol className="flex min-w-0 flex-wrap items-center gap-x-1 text-sm text-muted-foreground">
        {teamspace?.role && (
          <Crumb>
            <Link
              to="/w/$slug/teamspaces/$teamspaceId"
              params={{ slug: workspace.slug, teamspaceId: teamspace.id }}
              className="rounded-sm hover:text-foreground"
            >
              {teamspace.name}
            </Link>
          </Crumb>
        )}
        {page.owner_id === me.id && <Crumb>{copy.private}</Crumb>}
        {above.map((crumb) => (
          <Crumb key={crumb.id}>
            <Link
              to="/w/$slug/p/$pageId"
              params={{ slug: workspace.slug, pageId: crumb.id }}
              className="rounded-sm hover:text-foreground"
            >
              {crumb.icon ? `${crumb.icon} ` : ""}
              {titleOf(crumb)}
            </Link>
          </Crumb>
        ))}
        <li aria-current="page" className="max-w-[16rem] truncate text-foreground">
          {page.icon ? `${page.icon} ` : ""}
          {titleOf(page)}
        </li>
      </ol>
    </nav>
  );
}

function Crumb({ children }: { children: ReactNode }) {
  return (
    <li className="flex max-w-[12rem] min-w-0 items-center gap-1">
      <span className="truncate">{children}</span>
      <span aria-hidden>/</span>
    </li>
  );
}

function FavoriteToggle({ page }: { page: Page }) {
  const workspace = useWorkspace();
  const favorites = useQuery(favoritesQuery(workspace.id));
  const add = useAddFavorite(workspace.id);
  const remove = useRemoveFavorite(workspace.id);
  const favorite = favorites.data?.some((entry) => entry.id === page.id) ?? false;
  const busy = add.isPending || remove.isPending || favorites.isPending;
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={favorite ? strings.pages.favorite.remove : strings.pages.favorite.add}
      aria-pressed={favorite}
      disabled={busy}
      onClick={() => (favorite ? remove.mutate(page.id) : add.mutate(page.id))}
      className={favorite ? "text-primary" : "text-muted-foreground"}
    >
      <Star aria-hidden className={favorite ? "fill-current" : undefined} />
    </Button>
  );
}

function ShareButton({ page }: { page: Page }) {
  const workspace = useWorkspace();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <UserPlus aria-hidden />
        {strings.pages.share.open}
      </Button>
      {open && <ShareDialog workspace={workspace} page={page} open onOpenChange={setOpen} />}
    </>
  );
}

/** Over a page in the trash, for its editors: bring it back, or delete it for good. */
function TrashedBanner({ page }: { page: Page }) {
  const workspace = useWorkspace();
  const navigate = useNavigate();
  const restore = useRestorePage(workspace.id);
  // To the trash first: the page goes from the cache once deleted.
  const remove = useDeletePage(
    workspace.id,
    () => void navigate({ to: "/w/$slug/trash", params: { slug: workspace.slug } }),
  );
  const [confirming, setConfirming] = useState(false);
  const title = titleOf(page);
  const copyTrash = strings.pages.trash;
  return (
    <Alert live="polite">
      <AlertTitle>{copyTrash.banner}</AlertTitle>
      <AlertDescription>
        <p>{copyTrash.bannerLede}</p>
        {(restore.isError || remove.isError) && (
          <ErrorNotice error={restore.error ?? remove.error} />
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={restore.isPending || remove.isPending}
            onClick={() => restore.mutate(page.id)}
          >
            {copyTrash.restore}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={restore.isPending || remove.isPending}
            onClick={() => setConfirming(true)}
          >
            {copyTrash.delete}
          </Button>
        </div>
      </AlertDescription>
      <ConfirmDialog
        open={confirming}
        title={copyTrash.deleteTitle(title)}
        description={copyTrash.deleteLede}
        confirm={copyTrash.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          remove.mutate(page.id);
        }}
      />
    </Alert>
  );
}

function PageNotFound() {
  const workspace = useWorkspace();
  return (
    <PageHeader eyebrow={strings.notFound.eyebrow} title={copy.notFound} lede={copy.notFoundLede}>
      <div className="mt-3">
        <Button asChild variant="outline">
          <Link to="/w/$slug" params={{ slug: workspace.slug }}>
            {copy.home}
          </Link>
        </Button>
      </div>
    </PageHeader>
  );
}
